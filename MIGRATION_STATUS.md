# MCEdit2 Next — migration inventory

Status: 2026-10-21. The original Python 2 application remains in `src/mcedit2`
and `src/mceditlib`, but these files are **not** active in the Python 3 editor.
Statuses below describe working Python 3 equivalents, not whether old files
still exist. This is an engineering checklist rather than a release promise.

| Original MCEdit2 capability | New Python 3 implementation | Remaining gap |
| --- | --- | --- |
| Desktop editor and menus | Qt6 MainWindow, tool palette, World Info / Players / Chunk / Maps docks, history, minimap, queued imports and searchable Block Picker | Guarded world/player scalar and existing ItemStack edits; native Java filled-map pixel canvas and limited map metadata fields with independent gzip backup/restore; still lacks arbitrary item creation, modded slot editors, accurate modern map colors and original widget parity |
| 2D and 3D map views | 2D slice; OpenGL 3.3 Section streaming; Java blockstate variant/multipart JSON elements and rotated noncubic meshes while retaining greedy opaque cubes | Per-face translucent sorting, biome-aware fluid tint, animated texture playback, complex mod renderers, entity models, lighting and LOD (level-aware water/lava meshes are approximate) |
| Camera, picking and selection | Orbit, zoom, 3D ray selection, coordinate jump | Full MCEdit2 camera/navigation/selection widgets |
| Clone/move, copy, fill, replace, paint and undo | Native offset/repeat/rotate/mirror/scale/rotate-then-scale/reduction, independent exact-vanilla source-air and destination-air-only masks, rigid-group 3D wireframe/drag/grid snap, source ghosts when compatible; native BE/section preflight, grouped Undo/Redo and original region backups | True occupancy-aware masked 3D voxel preview, complex/fluid model parity, fractional transforms, modded orientation graphs, linked BE/entity moves and persistent history |
| Java 1.13+ Anvil | Native paletted Sections and guarded region rewrites | Newer codecs, full chunk heightmap/light/biome maintenance and real-world version matrix |
| Java 1.2–1.12 Anvil (.mca) | Native numeric IDs and metadata in original 16³ Sections | Mod-specific block geometry, lighting/heightmap updates, real-world game fixtures |
| Java Beta 1.3–1.1 MCRegion (.mcr) | Native XZY 128-height chunks, ID/metadata editing, backup and reopening | Old-style lighting/heightmap recalculation, real-world game fixtures | 
| Bedrock / Pocket Edition | No new LevelDB backend | Entire Bedrock adapter; will edit native LevelDB formats, not migrate worlds |
| BE NBT | Native NBT tree viewer, guarded scalar editing (types 1–6, 8) across modern/legacy Anvil and MCRegion, experimental clipboard | Array resizing/structural compound edit, mod-specific links/identity, safe BE moves |
| Entities / players | Original NBT inspector, guarded scalar/Pos/Rotation, group same-chunk translate and player ItemStack tools; native loose item/orb deletion, UUID-fresh cloning, original-UUID cross-chunk transfer and preflighted group cross-chunk offset transfers with one-step Undo/Redo and original region backups | General entity creation, mobs/vehicles, cross-dimension moves, passenger/owner graphs, modded identity/registry schemas and cross-entity references remain unported |
| Original Java filled maps | Qt6 native map_N.dat inspector, staging pixel paint/rectangle canvas, exact colors-array byte patches, whitelisted map metadata scalar edits, original gzip backup/restore | Full modern 208+ palette preview, modded map palettes, decorations/icons, world-side rendering fixtures, arbitrarily resizable map NBT and Bedrock maps |
| Vanilla .nbt structures | Block-only import/export | Entity/BE-aware structure transactions |
| Sponge v2/v3 .schem | Block-only import/export; strict nested Sponge v3 codec | BE/entity/biome payloads, rich metadata transfer and real-world fixtures |
| Alpha .schematic | Guarded numeric-ID/metadata subset | Unmapped IDs/rotations, BE/entity/ticks, complete metadata remapping |
| Other schematics | Qt6 Pending Imports queue for Java .nbt, Sponge .schem and Alpha .schematic, XYZ placement and single undo transaction | Zipped variants, complete BE/entity payload transfers, schematic previews with 3D overlays |
| Undo / revision history | Per-session undo/redo with migrated dock showing operations and BE/entity edit counts; backup browsing and guarded restore | Persistent journal, branching revisions and crash-atomic multi-region writes |
| World exploration | Region header index, asynchronous LRU Section streaming | Precise memory accounting, background priorities, large-scale profiling |
| Resource packs / model loading | Layered vanilla/mod/client ZIP/JAR resources, JSON parent textures, variants/multipart, noncubic elements, quarter-turn block rotation, limited diagonal element rotation, UV and cutout atlas | Per-biome tint, true translucency, animations, weighted alternatives, custom loader models and full texture compatibility |
| Plugins | Python 3 TOML plugins: commands, brush modes/shapes, tools, generators, read-only block/BE inspector callbacks | Legacy PySide1 widgets, preview scenegraph, schematic-based generators and complete original plugin APIs |
| Heightmaps, light, biome | Reader preservation only | Safe update and recalculation workflows |
| Formats and test matrix | Synthetic CI across 3 Python versions, Mesa and Qt | Genuine world fixtures with save/reopen across Minecraft versions/mod loaders |

## Safety/release gates

- No format is advertised as universal until unknown and modded content is
  preserved, or editing is rejected with an actionable error.
- Keep the draft PR unmerged; test only offline world copies.
- For world writes, require per-version fixtures plus Minecraft-side reload
  validation, especially block entities, lighting, heightmaps, and biomes.
- Vanilla Java 1.13+ paletted blocks are the only actively developed modern
  writer. Legacy numeric writers are constrained by each original format (MCRegion 0–255 IDs; preflattening Anvil 0–4095 IDs).

## Immediate migration work order

1. Validate classic `.schematic` and Sponge v2/v3 `.schem` against independent
   real-world fixtures; extend BE/entity/biome transfers only with lossless guards.
2. Expand actual-world fixture testing, verify no-loss chunk round trips and
   reject unknown codecs before mutation.
3. Extend block models/translucency and build UI for entity/BE inspection.
4. Design safe biome/light/heightmap updates with explicit version contracts.
5. Port original Qt tools, plugin dialogs, original history and more formats.
6. Extend the original-version backends with Bedrock LevelDB (no Anvil conversion).

## 原生格式编辑，不转换存档

每个存档始终使用其原始版本与底层格式进行编辑。不会对 `level.dat`、
`DataVersion`、旧版数字 ID 或区块整体结构进行自动升级。对于早于 1.13
的 Java Anvil 地图，可使用 `mcedit2:legacy_id_N[data=M]` 输入原生的
数字 ID（0–4095）与元数据（0–15），包括无法识别的模组 ID。
旧版 `.schematic` 导入同类旧版存档时无需转换成新版方块状态。

## Early Java MCRegion (.mcr)

Implemented native 16×128×16 MCRegion block editing in `mcregion.py`.
It reads and writes the original `Level/Blocks` and `Level/Data` X-Z-Y
arrays and uses existing 4-KiB sector-region I/O and backup logic without
creating `.mca` files. Original BE, entities, light, HeightMap and unknown
NBT are byte-preserved. Mixed ambiguous `.mcr`/`.mca` old worlds refuse
editing rather than guess which storage Minecraft will use. Only IDs 0–255
are valid in the original MCRegion format; Y is restricted to 0–127.

## Selection tools restored: clone, move, bounded flood fill

The Edit menu exposes block-only 3-axis offset cloning and moving, including
overlapping areas. Both snapshot the complete source selection before applying
any change. Moving clears source-only cells and populates destinations in one
undo step; selections follow successful moves. Contiguous 6-neighbor flood
fill is constrained to a selected volume, preventing accidental world-wide
filling of connected air. Region size and edit budgets apply.

These tools copy **native block states or legacy numeric ID/metadata as-is**,
without translating or rotating values. If a BE appears anywhere in the
source or destination of a move/clone, it is refused. Entities remain
unchanged; no entity or BE movement is implied. A native backend preflight
checks all target sections and IDs *before* staging any writes. Unsupported
or ungenerated destinations are rejected rather than creating chunks.

## 0.3.0a4 — Strict horizontal rotations and mirrors

The selection menu now offers clockwise and counterclockwise 90-degree
horizontal rotation plus East/West and North/South reflection. Both positions
and known vanilla blockstate properties are transformed, including facing,
horizontal axis, stairs handed corners, doors hinges, rail bends/slopes,
redstone direction-key connections and standing sign rotation.

The numeric pre-1.13 codec transforms only explicitly mapped historical
vanilla stairs, logs and torches; unknown legacy ID/metadata and unknown
namespaced mod blocks fail *before any edits*. All source/destination BE
positions are guarded and entities are not moved. The complete source volume
is captured before any writes, including rotated non-square volumes.
Operations are version-native: no flattening, DataFixer or world conversion.

More complicated multipart/tall blocks and mod-specific orientation fields
require tested per-version transformation profiles; refusing unsupported
states is intentional, not silently copying old orientation.

## 0.3.0a5 — Native entity position editing (restricted subset)

Added an opt-in GUI action to edit X/Y/Z position of a selected vanilla entity,
**within the same chunk**. The editor changes only the three original NBT
TAG_Double `Pos` values. It preserves all other entity tags, UUIDs and
mod-specific payloads byte-for-byte. The old `Level/Entities` and newer
standalone `entities/*.mca` formats are handled without version changes,
including regular undo/redo and region-file backup.

Only a short conservative allowlist of *simple unlinked vanilla entity IDs*
is writable. Entities with passengers, riders, leashes, attachment coordinates
or modded IDs are rejected, as are cross-chunk moves. All other entity data
remains read-only. This is **not** an arbitrary entity NBT editor and does not
validate runtime behavior of mods. Linked and cross-region migration are open.

## 0.3.0a6 — Type-preserving BE scalar editing

Selected **existing** BE scalar fields (Byte, Short, Int, Long, Float, Double
and String) can now be edited directly from the NBT tree without converting
the map. The editor replaces only the original tag's payload bytes, preserving
tag type, name, sibling compounds/arrays and unknown modded fields bit-for-bit.
Edits use the same undo/redo, backup and native save/reopen pipeline for
modern Java Anvil, pre-1.13 numeric Anvil and early MCRegion.

Protected identity/reference fields (including id, x/y/z, UUID, Owner and
Passengers) are refused, as are creating/deleting tags, replacing arrays or
compounds, converting NBT types and moving BE positions. Legacy BEs may be
changed ONLY in place. This is a low-level expert-only feature: edited numeric
fields may have mod-specific semantic limits that generic NBT cannot validate.
No default bulk rewrites or world version updates are performed.

## 0.3.0a7 — Existing NBT Byte/Int/Long Array member editing

Select a Byte_Array, Int_Array or Long_Array in the BE NBT tree and specify
an **existing zero-based index**, then change its signed value. The patcher
updates only 1, 4 or 8 original bytes and preserves the array count, tag
type and every other unknown/modded byte. Empty and out-of-range indexes
and protected UUID/owner/identity fields are refused.

Uses the same native undo/redo and backup/save pipeline for modern Anvil,
legacy numeric Anvil and MCRegion. Large arrays stay collapsed in the Qt
tree. Array resize, arbitrary compound edit and BE movement remain unported.

## 0.3.0a8 — Additional NBT structure editing

Existing BE NBT Compounds support insert/delete of simple scalar children;
existing scalar TAG_Lists and Byte/Int/Long Arrays allow indexed insertion
and deletion while preserving their original element kind and all untouched
bytes. Original BE identity, coordinates, ownership and UUID paths are
protected. All changes participate in undo/redo with original save backups.

This is expert-only structural editing: valid NBT mutations can still make
modded entities behave incorrectly. No full compound/list construction,
mod-specific schema inference or cross-version transformation is performed.

## 0.3.0a9 — Native region backup recovery

Added an explicit File-menu option to restore prior editor-created `.mca` /
`.mcr` region snapshots. Restoration requires no pending edits, validates
the snapshot is inside the same world's backup folder, refuses symlink and
unexpected files, checks the original `level.dat` and session lock, and
takes another full safety snapshot of the live target regions before writes.

Restoring does not modify `level.dat`, upgrade `DataVersion` or change region
formats. Undo history is reset and read/render caches are invalidated. A
failure during a normal multi-file write attempts rollback; crashes or power
loss mid-write are **not** transactionally atomic. External backups and
offline world copies remain required until crash-journal support is added.

## 0.3.0a10 — Native Python 3 generator and editor tool integration

Existing plugin registration categories `tool` and `generate` now execute
explicit Python 3 function callbacks from the Plugin menu, using the same
bounded EditTransaction and undo/rollback guarantees as plugin commands.
Generators require a selected bounded volume. A bundled checkerboard
generator demonstrates original-world block placement without conversion.

Old `registerToolClass` / `registerGeneratePlugin` class decorators are still
discovered but are NOT executed: their Python 2/PySide 1 constructor, old
Schematic return contract and original preview GUI require a deliberate port.
Third-party plugin code remains explicitly user-loaded and trusted, with
normal process/filesystem access; it is not a sandbox.

## 0.3.0a11 — Sponge Schematic v3 (block-only)

The Python 3 editor now imports both Sponge v2 and nested Sponge v3 `.schem`
files without modifying the target world's storage format or `DataVersion`.
Export remains v2 by default; the save dialog also offers explicit v3 output.
The v3 codec reads `Schematic/Blocks/Palette` and `Data` VarInts in
X-Z-Y order, preserving namespaced mod block states and the source schematic
`DataVersion`. It refuses any v3 biome container, nonempty entity/BE lists,
unknown block-container fields and nonzero offsets rather than discarding
unhandled content. The v3 reader shares the existing block-only version
mismatch confirmation and native-format world writer. Metadata descriptions
are not exported; no entity, BE or biome transfer is implied. Synthetic
regressions cover independent v3 fixtures, gzip round trips, malformed
payload guards and v2 backward compatibility. Genuine WorldEdit/MC fixtures
and Minecraft-side multi-version reopening remain outstanding.

## 0.3.0a11 — Inspector and brush shape plugin binding

Registered native Python 3 inspector callbacks can now inspect a block
state and unchanged optional opaque BE NBT payload at selected coordinates.
Output is displayed in a read-only Qt6 dialog; inspector callbacks receive
a snapshot, not an editing transaction. This is a convenience contract,
NOT a security sandbox: trusted Python plugins still run in-process.

The `brush_shape` registration category is also live. A Python 3 callback
receives `(PluginContext, point, block)` and may stage a spatial brush stamp
through ctx.set_block. Multiple stamps remain one undoable stroke. The
built-in plugin demonstrates a five-block cross shape and a native-block
summary inspector. Old PySide1 inspector classes remain registered but are
not automatically instantiated; they require an explicit Qt6 widget port.

## 0.3.0a12 — Native Qt6 plugin panel callbacks

The Python 3 plugin `widget` extension category is bound to the Plugins
menu. Only explicit callable providers are accepted. A provider receives
a Qt6 parent and returns a QWidget, displayed in a managed dialog;
invalid return types and exceptions are surfaced without leaving a panel.

Bundled selection-tools includes a trivial Qt6 panel example. Legacy
registered PySide1 widget classes remain discoverable but are NEVER
instantiated as Qt6. They require a manual rewrite. Full original
scenegraph/model previews and dock widgets remain unported.


## 0.3.0a15 — Read-only native selection statistics

Edit > Selection block statistics scans the bounded selection using the
existing world backend, with either original block IDs or complete native
BlockState / legacy numeric metadata groups. It reports exact block counts,
frequencies and the number of BE locations if the backend supports presence
queries. Unsupported BE counts show **unknown**, never zero.

Statistics never edit blocks or rewrite / normalize NBT, create undo history,
or save regions. Failed reads refuse to produce a partial report. The GUI caps
visible rows at 512 while totals are computed for all distinct states within
the existing edit-size selection limit; this does not remove the underlying
limit or claim safe editing of unsupported chunks.

Regression tests cover named block-state grouping, legacy metadata behavior,
modded BE original-byte preservation and Qt6 read-only dialogs. These tests
are synthetic; real modded Java world validation remains required.


## 0.3.0a16 — Native block-ID filter with BE-aware replacement preview

**Edit > Filtered replace by native block ID…** allows explicit source and
destination native states. By default it matches the namespaced block ID
across different properties/old numeric metadata; the **exact** checkbox
matches properties and legacy metadata too. The read-only preview counts
matching positions, proposed block writes and existing BE conflicts without
editing any world data.

Confirmation is required before committing one undoable operation. Any
matching block entity is a **hard failure for the entire operation**; it never
silently skips or rewrites unknown NBT. Backends without an authoritative BE
presence query refuse non-empty filtered writes. Native version-specific
preflight also rejects missing Sections and invalid old ID ranges before
applying changes. The exact same checks are performed again at commit time
so an outdated preview is not authoritative.

Synthetic and native save/reopen regressions include modded unknown BE NBT,
original paletted Anvil and numeric MCRegion, undo after save, and Qt6
dialog approval / cancellation and BE veto. There is no cross-version
format conversion and no claim of fully accurate relighting or support
for live server files.


## 0.3.0a17 — Python 3 read-only block predicates and native filter UI

Plugin registration adds the opt-in **filter** category: callbacks receive a
frozen FilterBlock(position, state, has_block_entity) and must return bool.
The Qt6 Plugins menu lists loaded Python 3 predicates, never unported old
PySide1 classes. Filter snapshots retain native states and numeric metadata.

The GUI previews selected/matching/proposed counts and BE conflicts, requires
confirmation, then re-runs the predicate and BE checks before a single native
undo transaction. Any BE overwrite or lack of BE presence checking rejects
the entire operation. Native preflight and original-format writer guards
remain in force. The bundled plugin demonstrates stone matching in both
paletted Anvil and numeric MCRegion.

Plugin, Qt6 and real native synthetic save-reopen tests cover bad results,
unloaded providers, no-BE-check refusal, conflict rollback, undo after save
and preserving opaque modded BE NBT. Trusted Python plugins still execute
in-process with user permissions. This is not a sandbox or complete legacy
filter/plugin parity; live-world fixtures remain outstanding.


## 0.3.0a18 — Configurable Python 3 filter options

Native predicate registration now includes typed option schemas via
PluginHost.register_filter(id, label, callback, options=[...]). This retains
the older single-record register_extension filter contract; configured
callbacks receive (immutable FilterBlock, validated options dict).
Supported option kinds reuse Qt6 command widgets: blocktype, choice,
int, float, bool, text, and display-only label.

Plugins > Python 3 native block filters shows a single Qt6 configuration
form for source/target block states, exact-vs-identifier matching, Y limits
and inversion in the bundled filter. The same validated options are passed
to read-only preview and the guarded rescan before commit. Invalid types,
unknown fields, out-of-range numbers, non-finite floats, unrecognized choices
and non-block arguments are refused; the internal target field is reserved.
Unconfigured legacy one-argument predicates continue to work as before.

Tests verify parameter handling, partial defaults, inversion, exact state
matching, Qt6 form interaction, undo, native modded Anvil save/reopen and
unchanged unrelated Block Entity NBT. This does not make Python plugins
sandboxed or equivalent to the full original MCEdit2 filter SDK.


## 0.3.0a19 — Dockable Qt6 tool palette (initial MCEdit2 tool UI port)

The editor now constructs a right-hand **MCEdit2 Tools** QDockWidget,
accessible from **View > MCEdit2 Tools**. It provides Paint/Select mode
switching, bounded original-format fill, hollow, flood-fill, line, clone,
move, rotation, mirror, search, statistics, copy/paste and filtered
replacement through the existing edit menu implementations. The dock's
native block ID/state field and selection Y bounds remain synchronized
with the editor toolbar; a manual refresh reports current selection
corners, volume, and the core operation budget without loading any chunks.

Loaded Python 3 tool, generator and filter providers are listed dynamically.
Plugin unload/reload rebuilds available actions, preserves a still-available
choice and otherwise selects a safe default. Opening the palette never
loads or executes external plugins. Plugin callbacks reuse their original
transaction/undo/BE guards; no alternate native writer was introduced.
The 2D Select mouse-release updates the selection summary automatically;
3D selections may use the explicit refresh control.

Qt6 offscreen tests exercise the native editable selection, toolbar/dock
field synchronization, undo, BE-conflict rollback, plugin load/unload,
and a generator called through the palette. GUI smoke now asserts the dock.
Remaining work includes full original MCEdit2 dock layout parity, context
tools, 3D interactive selection overlays, Bedrock, lighting/heightmap
updates and real-world multi-version fixtures.


## 0.3.0a20 — XYZ corner selection without solid blocks; parameterized shapes

The Qt6 MCEdit2 Tools dock now accepts two explicit three-dimensional
corners: X/Z corner inputs plus the existing synchronized selection Y bounds.
**Select XYZ corners** constructs a bounded selection for 2D and loaded 3D
viewports even when there is no solid voxel to ray-pick, including skyblock
voids and empty air. It validates integer coordinate ranges and the existing
core max-block operation budget **before changing viewport selection**.
This only changes UI state; it never reads, saves or generates a chunk.

The original 3D OpenGL base viewport emits selectionChanged on a completed
Select-mode mouse drag. The right-hand dock mirrors the resulting X/Z bounds,
while changing Y options does not overwrite an unfinished manual X/Z input.
Switching 2D/3D views, manually refreshing the dock, and opening a new world
also refresh coordinates. Opening a different world discards the previous
world's stale selection.

The dock now has contextual shape fields: shell thickness (1..64), sphere
radius (1..128) and a hollow-sphere toggle. Selecting these tools runs the
existing undoable EditSession.hollow_box/sphere operations with the chosen
parameters, instead of reimplementing native edits. Existing BE preflight,
per-operation budget, rollback and native format preservation remain active.

Offscreen Qt6 tests verify skyblock XYZ selection, selection budget rejection
without state changes, 2D/3D selection synchronization, 3D mouse drag
notifications and wireframe coordinates, ray misses, undoable shell/sphere
tools and unchanged world state during selection. Further work remains for
full original MCEdit2 tool panels, complex 3D editing gizmos, Bedrock LevelDB,
relighting/heightmaps, and diverse real-world cross-version fixtures.


## 0.3.0a21 — Interactive 3D selection gizmos and precision bounds

The original 3D OpenGL view and the streaming 3D renderer now draw six
orange face-diamond handles and a cyan center cross around selected block
bounds (Select mode). In a 3D view, **Alt+left-drag on a face handle** resizes
only that face; **Alt+left-drag on the center** moves the selected region
horizontally (X/Z); **Alt+Shift+left-drag on the center** moves it vertically
(Y). Handle picking uses camera rays and geometrical math without any
chunk reads or generation. The 3D view emits proposed integer-coordinate
bounds; MainWindow validates them through the same bounded
select_coordinates() method before updating either viewport. A rejected
oversized/out-of-range gesture leaves the selection unchanged and displays
a status-bar refusal. No world edit or undo operation is performed simply
by moving/resizing selection bounds.

The right-side tool dock also provides precise XYZ selection displacement
and single-face (X±, Y±, Z±) resize with integer offsets. It is now
scrollable so these controls remain accessible at reduced window height.
The base 3D mesh is explicitly invalidated when selection bounds change,
and streaming 3D draws the same gizmos through its existing GL_LINES pass.
Face shrinking clamps at one block instead of inverting the selection.

Headless geometry tests exercise every face, negative offsets, clamping,
closest-ray hit testing, X/Z plane and vertical-axis drags. Qt6 tests
exercise face handles, center X/Z and Y drag, precise dock controls,
volume-budget rejections and the scrolling palette. Disk-backed synthetic
original-format Java Anvil and MCRegion tests confirm that selection-only
transforms leave original region bytes, Block Entity NBT, staged edits and
undo history untouched. Complete original MCEdit2 3D gizmo parity and
representative real-world/modpack compatibility remain outstanding.


## 0.3.0a22 — Read-only original-world compatibility audit

A bounded forensic audit now scans existing Java region headers/chunks without
saving, staging edits, changing session locks, generating chunks, migrating
formats, or altering opaque Block Entity NBT. The native world adapter exposes
compatibility_audit(max_chunks=64, max_regions=32); hard limits are 512 chunks
and 128 regions. It reports region extension, level.dat DataVersion, sampled
chunk DataVersions, readable/damaged/unsupported chunk counts, observed BE
counts, missing/duplicate block Sections, invalid/cross-chunk BE coordinates,
mismatched chunk locations, damaged region sector offsets and overlaps, and
unsupported compression including external .mcc or LZ4. Mixed .mca and .mcr
region folders are explicitly reported as an excluded-format finding.
Per-chunk parse and decompression stay within existing native size limits;
reports cap detailed findings and distinguish partial scans.

The Qt6 File menu now includes **Audit native world compatibility
(read-only)…**, with an adjustable chunk budget and a copyable text report.
Audit is separate from Save and EditSession and cannot certify that a world
is safe to write: skylight, block-light, heightmaps, entity behavior,
modded invariants, unscanned chunks and full-version game correctness are
not fully validated. Do not equate a successful audit with full support.

Tests cover Java MCRegion, preflattening numeric Anvil, compact 1.13 and
padded 1.16-style palettes, modern 1.18+ layouts, BE duplication and foreign
coordinates, unknown/external compression, overlapping and invalid sector
headers, cross-region/chunk budgets, version mismatches, mixed extensions,
unchanged files and pending edits, plus Qt6 audit dialog/cancel/no-world
behavior. The test fixtures are synthetic representations of the original
formats; representative actual game-produced, modded multi-version worlds
remain a separate acceptance requirement.


## 0.3.0a23 — Derived lighting/heightmap impact and preservation guard

The native Java writer now verifies that all existing original lighting
and heightmap NBT tags remain byte-for-byte unchanged by every prepared
chunk patch. This includes legacy HeightMap, modern Heightmaps compounds
and child long arrays, section SkyLight/BlockLight arrays, isLightOn and
legacy LightPopulated/TerrainPopulated markers. A discrepancy refuses the
save before region-file replacement. Existing automatic region backups
and Block Entity / opaque tag preservation still apply.

A separate bounded read-only preview (NativeAnvilWorld.derived_data_impact)
reports staged changed blocks, touched chunk coordinates, changed X/Z
columns, edited section Y values, existing light/heightmap tag names and
structural anomalies (e.g. malformed light array sizes). The preview reads
disk chunks through the existing bounded reader without caching/staging
changes, and caps the scanned chunk count. The Qt6 File menu offers
**Inspect pending lighting / heightmap impact (read-only)…**, with copyable
plain-text details; Save now provides explicit potential-staleness warning
and change counts before the user chooses Yes/No (default No).

IMPORTANT: This phase verifies preservation, not automatic relighting or
heightmap recalculation. Changing a block can make preserved lighting or
heightmap values incorrect until an actual Minecraft engine or a future
verified version-specific updater processes the chunk. Modded emission,
opacity and block update rules are not inferred. This build does not
claim to fix the game's resulting visual or behavioral side effects.

Regression tests cover modern 1.18+ heightmap compounds and light arrays,
MCRegion legacy heightmaps, changed-column grouping, read-only scan
limits, dirty-edit preservation, deliberate metadata corruption detection,
modded BE equality, on-disk save/reopen, GUI impact report and rejection
of the experimental save confirmation. Actual game-generated modded and
cross-version save/load acceptance is not yet established.


## 0.3.0a24 — Native Java heightmap codec and no-write WORLD_SURFACE preview

Added a separate Java-native heightmap codec that decodes and encodes exactly
256 unsigned column offsets from Minecraft's original NBT long arrays.
Supported packing: pre-1.16 continuous bits across longs and 1.16+
non-crossing, padded long entries; the field width comes from an explicit
world build height and supports bounded custom heights. It verifies array
length, value range, unused bits, exact 256-value roundtrips, and both
older Level/Heightmaps and modern root/Heightmaps compounds. Original
HeightMap (numeric MCRegion/preflattening) remains separate; the new
paletted codec is strictly Java 1.13+.

A conservative, memory-only WORLD_SURFACE candidate reconstruction is now
available for up to 16 edited local X/Z columns in one chunk. It requires the
exact dimension min_y and world height to be supplied explicitly; the
editor does not guess custom datapack geometry. Before yielding a candidate,
it verifies the chunk DataVersion, block NBT, original stored WORLD_SURFACE
values against real original Section contents, and rejects any non-vanilla
namespace in the scanned edited columns. It refuses a missing WORLD_SURFACE
map, bad packed arrays, mismatched stale originals, cross-chunk edits,
oversized batches and invalid coordinates. It computes vanilla-style
topmost non-air per column (air/cave_air/void_air excluded) and returns an
immutable SurfacePreview containing a patched NBT **in memory only**.
A byte-span comparison proves no other root NBT bytes changed in the
candidate, including separate motion-blocking heightmaps and BE fields.

NativeAnvilWorld.preview_world_surface(cx,cz,min_y,height) applies this to
staged block-state edits without staging the resulting heightmap or
writing to disk. Qt6 File > Preview WORLD_SURFACE heightmap (memory only)…
prompts for exact height bounds and shows a copyable candidate report.
It never overrides the original preservation-only Save pipeline.

Synthetic tests cover 1.13 compact, 1.16 padded, modern 1.18+,
negative Y, a 512/1024 build height, roundtrip packing, inherited unknown
BE NBT, neighboring heightmap preservation, actual region bytes unchanged,
unknown modded blocks, incorrect source heights, missing tags, cross-chunk
edits and Qt6 preview/refusal. The in-memory reconstruction remains an
experimental building block, NOT a verified or enabled heightmap writer;
MOTION_BLOCKING, OCEAN_FLOOR, all lighting, custom block air semantics,
custom dimension geometry and real modpack gameplay require further work.


## 0.3.0a25 — Explicit guarded WORLD_SURFACE write within original-region save

The first, deliberately limited, opt-in native heightmap write path is now
available for offline test-world copies. Normal Save continues its existing
preservation-only behavior unless a user explicitly approves a reviewed
WORLD_SURFACE candidate for each block-edited chunk.

The backend offers approve_world_surface_update(cx,cz,min_y,height),
cancel_world_surface_updates(), and approved_world_surface_chunks.
Approval only records an in-memory reviewed preview and the exact native
staged block-state set, without modifying world chunks or NBT. Approval is
single-use: Save first confirms all block-edited chunks are approved and
then, during its existing locked backup-and-atomic-replacement preparation,
rechecks the original raw chunk and exact pending edits and independently
regenerates the candidate. It first performs ordinary block/BE/entity
patching with the original strict derived-NBT byte guard. It then replaces
*only* the approved WORLD_SURFACE long-array tag within the already patched
chunk and verifies that every other derived tag and all unrelated NBT bytes
remain unaffected. Saving still backs up complete original region files,
uses its existing best-effort multi-region rollback, and clears approvals
only on success. Invalid/stale proposals fail the whole save before any
region replacement; reverting or canceling approvals restores the default
preserve-only path. Remaining approvals also block backup restoration.
No-op save and editor close clear approvals.

For memory safety, approval is capped at 16 chunks with at most 32 MiB of
combined source/candidate NBT held in memory. Each chunk is also limited
by the prior 16-column strict WORLD_SURFACE preview. Source height and
build limits must be explicitly supplied by the user; unknown modded
states, inconsistent source heightmaps and cross-chunk coordinates are
rejected, not guessed.

Qt6 File now has **Approve WORLD_SURFACE update for current chunk
(experimental)…** plus **Cancel pending WORLD_SURFACE approvals**, separate
from the pre-existing read-only preview. The approval dialog requires
explicit world bounds and a separate warning/confirmation. The final
Save dialog independently warns when approved heightmaps will be written,
and still defaults to No.

Synthetic on-disk tests cover Java 1.13 compact, 1.16 padded, and 1.18+
WORLD_SURFACE writes, backups and reopen, exact modded BE/NBT and other
heightmap preservation, failed approvals, staged-edit changes, unapproved
chunks, restore refusal, no-op clears, and Qt6 approve/cancel/no-save.
THIS DOES NOT recalculate MOTION_BLOCKING, OCEAN_FLOOR, sky/block light,
or modded block updates. Actual-game acceptance for each specific version
and modpack is still required; production-world save safety is unproven.


## 0.3.0a26 — Fail-closed collision/fluid heightmap classification (READ ONLY)

Added two new, isolated, read-only heightmap analysis modules:
heightmap_predicates.py and heightmap_collision.py. For Java **1.21 and
1.21.1 release DataVersions 3953 and 3955 only**, they can compute bounded
MOTION_BLOCKING and OCEAN_FLOOR candidates from original native block
sections plus staged edits. Collision and fluid heightmap rules are not
interchangeable: MOTION_BLOCKING includes solid motion-blocking blocks and
non-empty fluid states; OCEAN_FLOOR includes the approved solid subset but
not plain water/lava. These predicates are tag-based in Minecraft; without
the running game's block registry, no arbitrary state can be classified
safely. The implementation explicitly permits only vanilla air, a small
enumerated group of simple full-cube solids and water/lava with validated
simple fluid levels. Leaves, slabs, stairs, transparent/non-full blocks,
waterlogging, unusual state properties, all modded namespaces and unknown
versions are *rejected* rather than guessed. In worlds that override vanilla
heightmap block tags with datapacks/mods, even the small static allowlist
may no longer match the game and must not be trusted.

The preview requires explicit dimension min_y/build height and an explicit
boolean acknowledgment that the relevant vanilla heightmap tag rules have
not been modified. It reads and validates the matching original packed NBT
heightmap, the exact chunk's DataVersion and X/Z identity, all original
Sections of the chosen explicit height range and every affected column;
then compares observed heights against stored originals before computing
a new candidate. A maximum of 16 affected X/Z columns per preview is
allowed. The candidate changes *only one named original heightmap tag*
inside an in-memory NBT byte array, preserving everything else, including
modded Block Entities. No world edits, saves or chunk generations occur.

NativeAnvilWorld.preview_collision_heightmap() is a diagnostic-only method
with no approval/writer integration. Qt6 File now offers **Preview
MOTION_BLOCKING / OCEAN_FLOOR (read-only)…** with explicit map choice,
world bounds and a separate default-No acknowledgment of unmodified
vanilla block tags. The existing WORLD_SURFACE opt-in save remains
unchanged; neither new heightmap can be written by the editor.

Regressions cover water-above-stone (raises MOTION_BLOCKING but not
OCEAN_FLOOR), solids, water-to-air edits, unsupported shapes/modded states,
invalid fluid properties, stale original maps, missing tags, mismatched
chunk coordinates, unacknowledged tag assumptions, unsupported data
versions, Qt6 read-only approval refusal and native disk-byte preservation.
All these are synthetic Java NBT fixtures; there are still no verified
real-game modpack registry/heightmap acceptance results. This is NOT
automatic heightmap rebuilding for all Minecraft versions.


## 0.3.0a27 — Version-bound exact-state collision heightmap rule evidence

Added a strictly parsed immutable JSON evidence snapshot interface to
extend the read-only MOTION_BLOCKING and OCEAN_FLOOR heightmap previews
without guessing unknown block properties. Each snapshot identifies a
single exact Java DataVersion, a claimed origin and a map of canonical
BlockState strings to **both** blocks_motion_in_heightmap tag membership
and nonempty-fluid-state booleans. No wildcard/substring/name-prefix
matching or automatic fallback is permitted once a snapshot is selected.
The importer accepts precise modded namespaces, leaves, slabs, stairs,
waterlogged property combinations etc. *only* when the exact state occurs
in the imported evidence: unknown states fail closed. An example/test
state classification is not proof of actual Minecraft runtime semantics.

The importer rejects malformed/duplicate JSON fields, invalid types,
noncanonical property ordering, unknown document fields, mismatched
versions and files larger than 4 MiB; state entries capped at 20,000.
Loaded facts are immutable, with claimed source and SHA-256 digest shown
in diagnostic output. It executes no Python, Java or plugin code and
reads no implicit mod/data pack paths. The new editor menu
**File > Preview collision heightmap using state evidence JSON…** explicitly
requests the file, heightmap name, exact dimension bounds and a default-No
acknowledgment that the user's supplied facts match their actual
runtime/modpack/tag configuration. This import supplements but does not
replace the previous built-in limited 1.21/1.21.1 classification, which
remains the default.

A matching evidence snapshot may support a different modern Java chunk
DataVersion than the built-in 3953/3955 allowlist; this is an experimental
exact-state diagnostic and cannot certify compatibility with that release.
The collision heightmaps remain **read-only**. It does not grant
WORLD_SURFACE write approvals or edit any original .mca/.mcr bytes.
Tests cover exact waterlogged stair/leaves/modded states, custom version,
unknown state refusal, malformed JSON, duplicate keys, file/symlink
limits, immutable state maps, Qt6 file picker, explicit confirmation,
fingerprint provenance and original-region-byte preservation.
See doc/heightmap_evidence.md for schema and safety limits.


## 0.3.0a28 — Optional Minecraft 1.21.1 NeoForge runtime evidence exporter

Added an optional source-only NeoForge 1.21.1 mod overlay under
tools/neoforge-1.21.1-exporter. The operator-only `/mcedit2export` command
queries the **actual running game** for all registered block states and the
Minecraft `Heightmap.Types.OCEAN_FLOOR` and `MOTION_BLOCKING` predicates,
plus `BlockState.getFluidState().isEmpty()`. It serializes exact canonical
property sets and the actual DataVersion 3955 into schema-1 JSON for the
existing read-only collision heightmap importer. It validates at runtime
that MOTION_BLOCKING equals (OCEAN_FLOOR or nonempty fluid), aborting if
that relationship ever differs. The JSON's historical
`blocks_motion_in_heightmap` key means the *effective OCEAN_FLOOR
predicate* for this exporter, **not membership in a Minecraft block tag**.
This corrects earlier imprecise documentation of that legacy key.

The export is only initiated by an authorized in-game command and writes
a content-addressed JSON file to `mcedit2-evidence/` under the Minecraft
game directory, not to a world save. It uses a same-directory temporary
file and an atomic rename where supported. Missing/mismatched game
versions, >200,000 block states, >32 MiB JSON and inconsistent runtime
predicates cause refusal without partial output.

The importer was expanded conservatively from 20k states / 4 MiB to
200k states / 32 MiB with a regression test importing 25,001 entries.
The exact-state matching and DataVersion validation are unchanged. No
heightmap writes are enabled by this feature, and ordinary native Save
still follows its existing safety rules.

An isolated, manually/labeled opt-in GitHub workflow now compiles the
helper against the official NeoForge 1.21.1 ModDevGradle MDK using Java 21;
the **first actual compiler CI passed** and produced a JAR inside the
build runner, with **no published binary**. This proves the Java source
compiles for the CI-selected NeoForge version, not that its command has
been exercised on a modded game server. Real runtime execution,
round-trip exported snapshots against game-generated worlds, and
cross-version/lighting safety validation are future acceptance gates.
See tools/neoforge-1.21.1-exporter/README.md.


## 0.3.0a29 — Verified real NeoForge server export and real game-world READ-ONLY audit

The optional official-MDK NeoForge 1.21.1 CI now goes beyond compilation:
a disposable headless Java 21 development server actually starts and calls
the same runtime-registry exporter behind two CI-only gates (JVM property
AND test environment variable). Production servers never auto-export:
the ordinary helper still requires the operator command /mcedit2export.

In the successful test run, Minecraft generated a genuine
DataVersion-3955 JSON covering **26,684 registered block states**.
The Python 3 importer verified its exact canonical state facts, full
SHA-256 filename/content match, source marker, GameVersion and
representative air, stone, water and waterlogged stair entries.
This is a game-generated vanilla+NeoForge runtime state registry,
NOT a snapshot from the user's installed modpack. No JAR binary is released.

After the server shut down and saved its test world, the existing native
MCEdit2 Java region backend opened a REAL Minecraft-generated 1.21.1
level.dat and .mca dataset and performed the first real-game read-only
compatibility audit: 4 region files, 168 header-indexed chunks and
48 successfully parsed/tested chunks in a deterministic bounded sample.
SHA-256 values of level.dat and every existing .mca file were identical
before and after the audit. The scan used no world mutators or plugin
callbacks, no staged editor edits, no partial region rewriting, and no
game-dependent serialization approximations.

GitHub CI run https://github.com/yanxiatao/mcedit2/actions/runs/39475914856
passed full Java compilation, real game runtime export, Python schema
handshake, and 1.21.1 real-region audit. Separate core tests now
exercise valid and invalid importer handshake inputs (digest, version,
source and required state completeness). The optional exporter workflow
remains independently triggered so everyday Qt6/Python validation stays fast.

Limitations: full-world scans, mods with real BEs, read/write/reopen in
the actual game, cross-version releases and modded lighting/heightmap
semantics have NOT been certified. A byte-identical READ-ONLY scan does
not guarantee that editing/saving such a map is safe.

## 0.3.0a30 — Original MCEdit2 feature-port batch (implementation-first)

Responding to the decision to prioritize **unmigrated features before
extensive testing**, this batch ports original Qt/metadata workflows in
the working `python3-modern-editor` branch. All migrated widgets reuse
the native Java NBT backend or existing EditSession editing transactions.
It does NOT bring old PySide1 modules into the active Python3 process.

**New dockable Qt6 modules:**
- `WorldPanelsDock` restores World Info (full original Data NBT viewer),
  player browsing and inventory/EnderItems ItemStack NBT inspection,
  bounded native Chunk Inspector, and map_N.dat preview+original NBT.
  No metadata/player/map writes, DataFixer conversions or guessed modded
  ItemStack schemas have been introduced.
- `NativeBlockPicker` is a searchable native-state selector: bounded
  current-chunk palette discovery and explicit user-selected runtime
  evidence JSON import. Selecting a state only changes toolbar input.
- `PendingImportsDock` restores the pending multi-structure workflow:
  queue .nbt/.schem/.schematic, edit per-item XYZ origins, review format
  and DataVersion, remove or clear entries. Applying merges last-wins
  overlapping destinations and uses one EditSession._commit, one Undo and
  native BE/format preflight; refused commits leave queue intact.
- `ShapeBrushDialog` migrates original square, round/ellipsoid, diamond,
  cylinder and parabolic dome shapes. Fill, replace-only, optional hollow
  shell, dimensions and center feed the existing native block transaction.
- `HistoryDock` exposes applied/redo operations and BE/entity edit
  counts using the same core undo stack, not a parallel transaction
  implementation. `MinimapDock` draws generated chunk occupancy from
  .mca/.mcr 8-KiB region headers with click-to-pan and zoom; no chunk
  generation or NBT decoding is caused by zoom.
- `map_panels_data` reuses the original MCEdit2 filled-map RGB palette
  for known legacy indices. Unsupported newer/opaque palette entries are
  visibly marked magenta and **not** passed off as exact modern colors.

**Not complete yet:** writable world metadata and inventory/map
properties, full modern map palettes and image import, all resource
pack model geometries, lighting/biome reprocessing, cross-chunk or
graph-aware entities/BE moves, all original Python2/PySide plugins,
complete Bedrock LevelDB support, format-specific future Minecraft
versions and a release-quality Windows binary. These are real feature
gaps, not things to mark as done by moving source files unchanged.

**Validation policy for this batch:** full CI and real-game save/reopen
checks have deliberately not been the focus of this migration-first
iteration. The previous 0.3.0a29 CI is NOT a claim that these new Qt6
modules were tested. Keep Draft PR unmerged and use offline test worlds.


## 0.3.0a31 — Resource-pack noncubic model geometry migration

The source-only feature migration now extends the real streaming renderer,
not merely the model inspector. Added `modern/mcedit2_next/block_models.py`:
strict, bounded, independent Java JSON model resolver reading the already
layered client JAR/resource pack/mod JAR `ResourceLibrary` assets. It
combines `variants`, `multipart` with AND/OR property conditions,
bounded model parent inheritance, merged texture variables, each element's
from/to coordinates, face texture and per-face UV rectangle/rotation,
quarter-turn blockstate X/Y rotations, and supported element rotations
(-45/-22.5/0/22.5/45 degrees including the JSON `rescale` and `origin`
fields). Zero-thickness plane elements are retained for foliage-style
crossed geometry. Model parsing never loads or alters original world NBT.

`StreamingAtlas` now maintains resolved special geometry and stable
per-face texture tile IDs, passing immutable numeric model faces to
`MeshBuildPool` on background workers. `greedy_section_geometry` keeps
its old efficient rectangle merging for regular full cubes, but excludes
noncubic/rotated/custom-UV states from fake solid-cube geometry and appends
true model faces; neighbor special models no longer suppress exposed
ordinary cube faces. External face culling is conservative, and a bounded
special-geometry vertex count prevents massive per-section allocations.

The OpenGL shader drops fully transparent atlas texels, avoiding fully
opaque square backgrounds for basic alpha-cutout cross-plants and pane
textures. Models outside this constrained vanilla JSON subset still use
the existing unmistakable full-cube diagnostic fallback. This is **not**
full parity with the original MCEdit2 scenegraph or Minecraft renderer:
animated textures, biome-color models, accurate fluid transparency,
sorted translucency, overlay/model-display transforms, weighted variant
randomization, entity rendering, non-vanilla custom model loaders and
precise subvoxel picking remain outstanding.

Following the user's implementation-first direction, **the a31 renderer
changes have not undergone full CI or actual OpenGL screenshots/game-view
comparison yet**; prior a29 green results do not cover a30/a31. Keep PR
Draft, master unchanged, no Windows preview release.


## 0.3.0a32 — Streaming two-pass translucency migration (implementation-first)

Added `render_layers.py`: conservative classification of known Minecraft
water, stained/clear glass and selected translucent vanilla states, plus
per-state resourcepack textures containing **partial** alpha. Unknown
modded render-layer types are not guessed from names. Section keys can be
sorted back-to-front using camera distance without rereading chunk NBT.

`StreamingAtlas` now uses Qt QPainter `CompositionMode_Source` when
inserting PNG tiles, **preserving their original alpha** rather than
flattening transparent pixels against the original white atlas. It
tracks which texture tiles contain genuinely partial alpha and passes a
snapshot to the off-thread mesh classifier.

The existing `greedy_section_geometry` retains its legacy flat-list API,
but accepts optional `split_layers=True` with explicit translucent states:
opaque triangles and translucent triangles are emitted separately while
full-cube greedy merging, model-specific triangles, native blockstate
identity and external-face culling are preserved. Interfaces between
opaque blocks and translucent materials no longer hide the opaque
background face; adjacent identical water/glass blocks suppress interior
faces. `MeshBuildPool` supports a compatible optional two-byte-payload
mode without changing the old default byte-returning worker API.

`StreamingVoxel3DView` stores both layers in **one GPU buffer per
Section**, draws opaque vertices first with normal depth writes, then
draws the translucent subset with `GL_SRC_ALPHA /
GL_ONE_MINUS_SRC_ALPHA`, `glDepthMask(GL_FALSE)` and **coarse
back-to-front Section ordering**. The selection overlay draws after
normal depth writes are restored. Layer opacity is adjustable through
**View > 3D translucent opacity…**, persisted in QSettings, with no
Section invalidation or world I/O. The standard alpha-cutout shader
discards near-zero-alpha texture pixels in both passes.

New focused `test_render_layers.py` and `test_translucent_atlas_qt.py`
files document expected transparent/solid adjacency, transparent
neighbor Section culling, worker backward compatibility, texture alpha
retention and render-layer classification. Consistent with the user's
explicit migration-first choice, these tests and full Mesa/Qt CI **have
not yet been run on a32**. Prior green a29 workflows do not validate
this source batch.

Remaining renderer gaps: ordering is by Section center, **not per
translucent face or fragment**, so intersecting glass/water may still
produce artifacts. The global blended opacity is an approximation;
Minecraft water-level surface geometry, biome/fluid tint, shader light,
animated textures, refractive glass, weighted model alternatives and
modded custom render types are NOT equivalent to the in-game renderer.
No native map files or BE data are modified by these rendering changes.

## 0.3.0a33 — Level-aware vanilla fluid surfaces (implementation-first)

The streamed Qt6 3D viewport now constructs render-only sloped water/lava
surfaces for **exact** `minecraft:water` / `minecraft:lava` blockstates
with valid `level=0..15` values. The native state, Section NBT and region
files remain unchanged. Source and flowing water levels use bounded corner
height interpolation, clipped side quads, removal of connected-fluid
interior faces, and six-cardinal-neighbor sampling. Diagonal neighbor
Sections are not available, so corners never invent those samples.

The fluid path is threaded through the existing two-pass greedy Section
mesher, rather than a separate Qt render world. Ordinary opaque cubes,
unknown modded block IDs, original custom model faces, old default mesh
APIs and BE-saving safeguards retain their previous code paths. The
resource atlas resolves vanilla still/flowing fluid tiles when available;
for common vertically stacked animated PNGs it displays only the first
unscaled frame. Every state level of one fluid shares those stable tiles.

Added focused `test_fluid_geometry.py` and fluid-asset Qt tests for
source/flowing/falling levels, neighbor borders, opaque adjacency, and
backward compatibility. They are **committed as regression specifications**,
not asserted to be passing; full Mesa, Qt, integration CI, and Minecraft
runtime/game-save tests remain deferred under the user's migration-first
direction.

**Limits:** This is not the exact Minecraft fluid tessellator. Corner
weighting and cross-boundary sampling are conservative approximations;
no biome-aware fluid tint, waterlogged-subshape blending, animation
playback, custom modded fluid renderers, per-face translucent sorting,
or lighting propagation has been completed. Bedrock LevelDB and the
remaining legacy Python2 editor capabilities are still outstanding.
Keep Draft PR #1 open and unmerged; no Windows preview artifacts.

## 0.3.0a34 — Native original Java world/player scalar editing and metadata restore

Ported selected original MCEdit2 World Info and Players **write** controls into
the Qt6 `WorldPanelsDock`, without loading legacy PySide1. New
`metadata_edit.py` operates directly on existing **Java gzip NBT files**
(`level.dat`, the `Data/Player` singleplayer compound or canonical
`playerdata/<uuid>.dat`) rather than reconstructing world formats.

- Explicit **existing scalar whitelist**: world name, game mode, difficulty,
  command/hardcore flags, spawn X/Y/Z, game time and selected world-border
  numerics; player health, food, XP, selected hotbar slot, air/fire and
  invulnerability. Original NBT tag types must match each supported property.
  No arbitrary NBT field, inventory, EnderItems, ItemStack or modded player
  state insertion/deletion is enabled. Version-incompatible/missing tags
  are rejected without guessing new schemas.
- Uses the existing byte-span NBT scalar patcher and typed GUI conversion.
  Every unrelated parsed NBT tag **remains byte-identical** in the
  uncompressed original NBT. The complete gzip stream is recompressed, but
  the complete compressed original is archived before replacement. There is
  **no world DataVersion upgrade**. The native world's level.dat fingerprint
  and spawn readout are synchronized only after an actual committed edit.
- Immediate commits require **no pending EditSession region edits or
  heightmap approvals**, matching world identity, unchanged
  `level.dat`, unchanged displayed metadata snapshot, unchanged
  original source file while preparing, original-world session.lock
  ownership where supported and safe non-symlink paths. Each commit produces
  a new original compressed NBT backup under
  `.mcedit2-backups/<timestamp>-meta-<id>/...`, then an atomic
  same-directory replacement retaining file mode.
- Added **Restore original metadata backup…** to the World Info dock.
  Only program-created metadata snapshot names and original target
  paths are eligible; another inverse backup of the current target is
  required before restoration. Restoring `level.dat` refuses a DataVersion
  mismatch. The same session-lock, stale metadata and pending-edit checks
  apply. Region backups and player-data backups remain separate.
- These metadata edits are **deliberately outside** the existing region
  EditSession Undo/Redo history. Use the explicit backup restore control
  for reversal. This avoids claiming an atomic transaction across
  unrelated chunk and metadata writers. World files must still be **offline
  disposable copies**; lock behavior on all network filesystems cannot be
  guaranteed.
- Added `tests/test_metadata_edit.py` source-level regression
  specifications for preserving modded NBT, inventory bytes, compressed
  backup snapshots, stale-dialog/session-lock refusals, world position
  refresh, inverse restoration and unsafe target rejection. As requested,
  the full Linux Qt/Mesa CI and game-side compatibility matrix remain
  **deferred**. No assertion of a passed suite or Windows build.

**Remaining**: full player inventories/item counts, unknown modded
metadata schema editors, advanced world generators, GUI metadata
widgets, native map image writing, entities/BE graph moves, Bedrock
LevelDB, full animation/render parity, modern chunk codec coverage,
lighting/biome recalculation and live Minecraft version fixtures.
Keep Draft PR #1 unmerged, only `python3-modern-editor` touched.

## 0.3.0a35 — Original-format player ItemStack count / empty-slot editing (implementation-first)

Expanded the native Java metadata writer and original MCEdit2 player Qt6 dock
with a narrow, explicit **existing ItemStack** edit path. This is not a new
item serializer or a cross-version inventory translator.

- Supports existing entries in native `Inventory` and `EnderItems` on
  the selected original singleplayer `Data/Player` or canonical
  `playerdata/<uuid>.dat`. The selected tree row stores the exact original
  NBT `TAG_List` member index; an index is **not** conflated with the
  numeric `Slot` key.
- **Edit existing stack count…** changes only the existing original numeric
  `Count` (legacy) or `count` (modern) NBT value at a specified list
  member, preserving its original Byte/Short/Int tag width. Conservative
  supported counts are 1–127; no creation of empty stacks, identity
  changes, modded component reserialization, or guessed DataFixer defaults.
- **Move to empty native slot…** changes only that entry's original numeric
  `Slot` value within its current container. Standard Inventory slots
  0–35, 100–103 and -106 (legacy signed-byte offhand) and EnderItems
  0–26 are accepted. No occupied target, duplicate existing Slot,
  mod-added slot layout, container crossing, deletion, or wholesale
  inventory rewriting is accepted.
- Item `id`, modded `components`, nested custom NBT, enchantments,
  other item entries, the list shape and the world `DataVersion` remain
  unchanged at original uncompressed NBT byte-span level. The original
  NBT `TAG_List` entry identity and count shape are validated before and
  after the change. Unknown numeric field types, ambiguous dual
  `count`/`Count`, unsupported slots, invalid data and noncanonical
  `playerdata` names are rejected.
- Both operations use the **same existing guarded metadata commit**
  mechanism as 0.3.0a34: offline Minecraft `session.lock` check,
  `level.dat` external-change guard, stale original player snapshot
  comparison, no pending chunk/history/heightmap edits, compressed NBT
  backup per write, atomic same-file replace and prior metadata-backup
  restoration. These edits are **not** included in block-region Undo/Redo.
- Added tests to `tests/test_metadata_edit.py` as source-level
  specifications for original modded stack NBT preservation, Byte/Int
  counts, empty/offhand slot moves, duplicate/occupied slot refusal,
  stale dialog prevention and original gzip backup/restore.

**Not claimed ready for valuable/live worlds:** Full Qt/Mesa Linux CI,
real-game inventory round trips, modern snapshot-specific item schemas,
modded containers and genuine Java/Bedrock multi-version validation are
still pending under the migration-first workflow. No Windows build; Draft
PR #1 stays unmerged. Further inventory parity needs whole-stack
creation/deletion/transfer and inventory layout validation by version.

## 0.3.0a36 — Loss-averse native ItemStack swaps and Inventory↔EnderItems transfers

Ported the next original player-inventory workflow to Python 3 and Qt6:
**swap two occupied native slots** or **move an existing stack between
the same player's `Inventory` and `EnderItems` original NBT lists**.
The new `inventory_transfer.py` is a pure original-byte-span NBT backend;
the Players dock exposes **Transfer / swap existing ItemStack…** with an
explicit confirmation identifying an occupied destination and swap partner.

- Single-container swaps update only the two existing numeric `Slot` tag
  payloads at their precise original byte offsets. Empty-slot movement
  updates exactly one original numeric `Slot` tag; it does not rearrange
  other item entries.
- Cross-container moves retain the entire original item Compound payload
  **byte-for-byte except `Slot`**, remove it from the original NBT List,
  and append it to the existing destination List. An occupied destination
  can be exchanged explicitly: the two complete original stack payloads
  switch containers/slots in one original-file patch. Every unaffected
  entry and unknown player/mod tag is preserved.
- Both inventory lists must already exist. All participating list entries
  must match recognized standard Inventory slots (0–35, 100–103, -106
  offhand) or EnderItems slots (0–26), with unique original numeric
  `Slot`, original `id` and one valid `Count` or `count` field.
  Any unknown custom slot layout, invalid/missing/duplicate slot, ambiguous
  count field, unrecognized NBT layout, over-limit list or unauthorized
  occupied-slot overwrite is rejected.
- The writer reuses the exact 0.3.0a34–a35 metadata commit path:
  offline session lock, same-world `level.dat` fingerprint check, stale
  player snapshot protection, no pending chunk edits or heightmap
  approvals, complete original gzip file backup and atomic replacement.
  The earlier inverse metadata restore UI can undo any successful transfer
  by restoring the original compressed file. This is **not** part of
  region Undo/Redo and is **not** a cross-player transfer.
- `tests/test_inventory_transfer.py` contains source regression cases
  covering byte-identical opaque components, same-list and cross-list
  swaps, empty-list insertion, unsupported slots, no-accidental-overwrite,
  roundtrip transfer and guarded disk save/inverse restoration.

**Incomplete:** item creation/deletion, stack splitting/merging,
modded slot layouts, arbitrary inventory item edits, native map updates,
linked entity graph editing and version-specific modern component/item
schemas. Source tests are committed, **not claimed executed/passed**;
full Qt/Mesa CI, world/game reload tests and Windows packaging remain
deferred until the broader migration is ready. Only existing Draft PR #1
and `python3-modern-editor` are modified.

## 0.3.0a37 — Existing original ItemStack split, strict merge and deletion

Continued migrating MCEdit2's original-player inventory controls without
inventing item registry data or normalizing modded ItemStack NBT. New
`inventory_stacks.py` builds on the native, backup-protected `Inventory`
and `EnderItems` edit and transfer paths.

- **Split existing stack into an empty native slot:** clone the precise
  original ItemStack Compound byte payload, changing only its native
  `Slot` and numeric `Count`/`count` fields, and reduce the original
  stack's count. The original stack must keep at least one item. A
  cross-container split works within the *same player* only and only
  when both native containers already exist. Unknown mod components,
  enchantments, custom tags and unrelated list members are preserved.
- **Merge into a strictly identical existing stack:** require an occupied
  target with the same original `Count` tag name and NBT type and
  **byte-identical entire ItemStack payload after removing only numeric
  Slot and Count payloads**. Other modded metadata, identities, components,
  nested tags and layout must match exactly. Full merge removes the
  exhausted original list entry; partial merge leaves the exact
  original metadata and decrements its count. No cross-player merge.
- **Delete selected original stack:** explicitly remove one existing
  original NBT List Compound and preserve every other stack's original
  member bytes. An empty Compound List remains valid. This action
  requires explicit confirmation because it destroys the selected
  entry's contents.
- Same native whitelist as a35/a36: standard Inventory (0–35,
  100–103, signed-byte offhand -106) and EnderItems (0–26), unique
  slots, existing original ID/Count fields, conservative numeric count
  1–127, no new item identifiers or arbitrary modded container schemas.
  Invalid slots, non-identical stacks, invalid count ranges, occupied
  split destinations and absent original lists are refused without
  changing the world.
- Qt6 Players dock adds **Split / merge existing stacks…** and
  **Delete selected existing stack…**. It discloses that there is *no*
  version/mod-aware item registry lookup: Minecraft may use a smaller
  per-item maximum (including unstackable items). This workflow is
  experimental and must be used only on disposable offline world copies.
- Writes share the guarded original `level.dat`/playerdata backend:
  Minecraft session lock, stale dialog/fingerprint refusal, no pending
  region edit transaction, complete compressed file backup and atomic
  same-file replacement. **Inverse restore via original metadata backups**
  is available. These changes do **not** participate in chunk
  Undo/Redo and are not claimed cross-file atomic.
- Added `tests/test_inventory_stacks.py` source regression cases for
  same-/cross-container split and merge, full source removal, original
  lowercase Int count, untouched modded fields, mismatched stacks,
  invalid values and backup/restore.

**Remaining:** registry-driven arbitrary ItemStack creation, broader mod
inventories, version-specific item-component UI editors, native filled
map painting/metadata edits, linked entity migrations, Bedrock LevelDB and
other outstanding renderer/plugin parity. Tests have been **added but
not run in a full Qt/Mesa/CI/game-world matrix** as requested. No
Windows build, new PR, branch, merge or master changes.

## 0.3.0a38 — Native original Java filled-map pixel canvas and guarded metadata

Ported original MCEdit2 filled-map editing beyond the earlier **read-only**
`map_N.dat` viewer. This migration operates **only on existing original
Java gzip map files** under `world/data/`; it does not upgrade DataVersion,
invent modern map palette colors or touch chunk region files.

- New `native_map_edit.py` parses the original `data/colors` NBT
  TAG_Byte_Array, validates width, height, exact array length and the
  existing NBT tree, then patches **only changed pixel bytes** through
  native NBT byte spans. The complete original map metadata, unknown
  modded tags, unrelated array entries and original palette byte
  values (including unsupported modern colors 208–255) remain intact.
  Duplicate/out-of-bounds/invalid pixel requests fail before writing.
- Qt6 **Paint original map pixels…** opens a staging-only interactive
  `map_painter.py` canvas with mouse click/drag, native byte palette
  selector, color sample, exact coordinate painting, bounded rectangle
  fill and discard/revert staged changes. Explicit Save confirmation
  is required before any write. The older MCEdit2 legacy palette is used
  for previews. **Unsupported modern palette codes show diagnostic colors,
  not invented 1.21+ colors.** The actual stored bytes are not converted.
- A new **Edit original map scalar NBT…** action reuses the existing
  NBT tree dialog but whitelists only existing matching native
  `scale` (Byte, 0–4), `locked`, `trackingPosition`,
  `unlimitedTracking` (Byte flags 0/1), and `xCenter`/
  `zCenter` (Int) metadata tags. Original `colors`, dimension,
  width/height, decoration entries and unknown mod data are never
  editable through this generic scalar pathway.
- Editing requires the Java native backend, a selected existing regular
  `data/map_N.dat`, an unchanged original world `level.dat`
  fingerprint, compatible session.lock and **no pending region edits**
  or heightmap approvals. Stale UI snapshots are compared against the
  original `data` NBT payload on disk before committing. The original
  gzip file receives an exact compressed **backup first** in an
  `.mcedit2-backups/<timestamp>-map-<id>/data/map_N.dat` snapshot;
  replacement is an atomic same-directory original file write.
- **Restore map backup…** browses only validated editor-generated
  original-map backup paths, validates current/restored map dimensions,
  refuses symlinks, files missing from the original world and external
  level.dat changes, then archives a fresh inverse backup before
  restoring the exact compressed source file. This is explicitly **outside
  the region EditSession Undo/Redo system**.
- New `tests/test_native_map_edit.py` supplies source-only synthetic
  regression cases for pixel byte isolation, custom modded map field
  preservation, unsupported palette-byte round-trip, metadata whitelist,
  invalid geometry/coordinates, no-op, stale view, locks, gzip backup
  and inverse restore. These tests were **not executed** under the
  migration-first development plan.

**Still missing**: accurate version-aware modern map color table and
modded map palettes, decorated-map icon/marker editing, importing full
palette images with guaranteed game color equivalence, changing map
format/dimensions, Bedrock LevelDB maps, and actual Minecraft-version
save/reload verification. No full Qt/Mesa CI pass or Windows packaging
is claimed. All changes remain on Draft PR #1 / `python3-modern-editor`.

## 0.3.0a39 — Native entity yaw/pitch and grouped within-chunk transforms

Continued the Python 3/Qt6 original MCEdit2 entity editor migration by
reusing the existing **native Java original-format entity save writer**.
No world/UUID conversion or cross-file entity relocation was added.

- `entity_edit.py` now reads an existing two-member `Rotation`
  `TAG_List<TAG_Float>` and supports standalone whitelisted vanilla
  entity yaw/pitch mutations by rewriting only the original two Float
  payload bytes. Non-finite angles, unsupported/linked entities, absent
  or incompatible Rotation lists and out-of-bounds values fail closed.
  Original `Pos` (three Double values), entity ID, UUID, unknown mod
  NBT, passengers and all unrelated tag bytes remain unchanged.
- The native entity region/inline-entity chunk writer validates
  **Pos and optionally Rotation** differences without changing
  `patch_entity_positions`'s default Pos-only validation API.
  Legacy entities **without** an original Rotation list remain eligible
  for their prior Pos-only in-chunk edits; a Rotation field is never
  invented. Staging can include both original Pos and Rotation for
  one entity in the same chunk and the same save.
- `entity_index.py` exposes optional read-only yaw/pitch in the entity
  record. `entity_inspector.py` now shows those values and adds
  **Edit selected entity yaw/pitch…** alongside existing position
  editing and original NBT inspection. Both operations write through
  native `EditSession` staging, one-action Undo/Redo, dirty-state
  tracking, original region backup and save/reopen semantics.
- **Move selected entities by offset (single undo)…** adds Qt6 list
  multiselection and a bounded XYZ translate form. The core validates
  the *entire* requested batch up front by asking the native backend
  to test each candidate against the source entity and its current
  storage chunk **without staging any write**. Invalid source/index,
  duplicate entity selection, unsupported or linked NBT, cross-chunk
  targets, nonnumeric input or more than 2048 selected members rejects
  the batch. If all pass, one EditSession history entry groups the
  individual Pos changes, with rollback on an unexpected partial
  failure.
- Source regression file `tests/test_entity_rotation.py` specifies
  two-Float byte preservation, rejection of modded/linked entities,
  dual Pos+Rotation edits, original Pos-only legacy compatibility,
  grouped transaction preflight, single-undo behavior and original
  separate `entities/*.mca` region save/backup/reopen. The
  full Qt/Mesa CI, Minecraft-side reload and Windows packaging were
  **not run** as the migration-first workflow is still in progress.

**Remaining**: cross-chunk and cross-region entity migration,
passenger/root-vehicle/leash UUID graph moves, native entity create/
delete, complex riding and display item/model rendering, broad
custom mod entity support, legacy plugin UI parity and complete
version-by-version validation. Draft PR #1 stays open and unmerged
on `python3-modern-editor`, with `master` unchanged.

## 0.3.0a40 — Guarded original Java entity scalar NBT editing

The migrated Qt6 entity inspector now includes an opt-in **Edit existing entity
scalar NBT…** action, backed by strict native Anvil/MCRegion original-byte
patching rather than arbitrary NBT deserialization and reconstruction.

- New `entity_scalar.py` explicitly whitelists only *existing,
  correctly typed, top-level* `NoAI`, `Silent`, `Glowing`,
  `Invulnerable`, `NoGravity`, `CustomNameVisible`,
  `PersistenceRequired` (Byte Boolean flags 0/1), plus existing
  `Health` and `AbsorptionAmount` (Float 0–2048). No new fields
  are synthesized; unrecognized historical kinds remain opaque and
  unchanged, even if the entity's other original Pos fields are edited.
  No version-specific mod data or entity registration is guessed.
- Only the existing **simple standalone vanilla entity** allowlist is
  editable. Unknown/modded IDs, `Passengers`, `Riding`, `Leash`,
  `Attach`, ownership/root vehicle and other relationship fields
  reject edits. The original Pos must belong to its existing storage
  chunk; identity, UUID, world version, map state and all other NBT
  remain immutable.
- The scalar editor uses the original `nbt_scalar.patch_scalar`
  byte-span patcher, preserving tag kind and every unrelated
  original byte. Native `patch_entity_positions` now has an
  **opt-in strict `allow_scalar`** verification path for the
  actual Anvil region staging/saving code. Before on-disk write,
  it revalidates the exact original entity structure and compares
  all bytes except existing permitted scalar, Pos Double and
  Rotation Float payload bytes. Arbitrary nested, modded or
  UUID changes cannot accompany a scalar edit.
- `NativeAnvilWorld` now exposes `get_entity_scalar` and
  `set_entity_scalar`, integrated with the existing staged
  `_entity_edits` dictionary. `EditSession.edit_entity_scalar`
  creates a single Undo/Redo record per accepted operation and
  coexists with previous Pos/Rotation and multi-entity edits.
  The existing full-original-region backup precedes save.
- `EntityListDialog` shows the existing NBT tree with a gated
  scalar edit callback. The standard inspector may display many
  original scalar tags, but **only the native whitelist** is
  writable; unsupported entries produce an explicit refusal.
  Saved edits refresh their current original entity payload.
- `tests/test_entity_scalar.py` adds source-only specifications
  for exact untouched UUID/opaque component bytes, Byte and Float
  bounds, absent/wrong-typed fields, linked/modded source rejection,
  unknown NBT hitchhiking prevention, composability with native
  Pos/Rotation edits, EditSession Undo/Redo and separate
  `entities/*.mca` backup/reopen.

**Limitations:** This is not arbitrary entity NBT editing or a full
entity creation/deletion/cross-chunk migration editor. Entity-specific
game limits, linked graph handling and modded entity schemas remain
unimplemented. New tests are **committed but have not been run** in
the full PySide6/Mesa/CI or in-game version validation matrix.
Windows packaging remains disabled. Only existing Draft PR #1 /
`python3-modern-editor` is changed; `master` stays untouched.

## 0.3.0a41 — Native original dropped-item and XP-orb entity deletion

Expanded the Python 3/Qt6 MCEdit2 entity inspector with **guarded original
Java entity List member removal**, retaining the original unknown NBT of
all survivors and respecting world storage layout.

- New `entity_delete.py` implements byte-preserving removal from existing
  `Entities` TAG_List<TAG_Compound> under either modern root or legacy
  `Level/Entities`. It reconstructs only the original NBT List's
  element count and concatenation of unchanged original member payloads.
  Unrelated world/chunk NBT and each survivor's original Entity compound
  remain **byte-for-byte unchanged**.
- **Intentionally narrow original entity allowlist:** only existing
  unlinked `minecraft:item`, `minecraft:experience_orb`, legacy
  `Item` and `XPOrb` from their actual original storage chunk. Any
  Passenger/Riding/Leash/Owner/root-vehicle or other known linkage,
  modded IDs, mobs, vehicles, malformed positions, invalid/duplicate
  list indices or wrong chunk assignments are refused; no assumed
  UUID graph rewrites or cross-chunk migration.
- Native `NativeAnvilWorld` now retains a set of *original list indices*
  for pending removals, separate from the prior indexed entity NBT edit
  dictionary. Its Qt6 entity-list browser filters staged deletions
  without renumbering surviving `EntityRecord.storage_index` values;
  all other entities remain selectable/editable against their original
  positions even when lower-numbered entries have been staged for
  deletion. Deleting a record with pending original scalar/Pos/Rotation
  edits is refused, preventing contradictory staged mutations.
- `EditSession.delete_entities_in_chunk` preflights the **entire
  bounded selection** before staging it as one history entry; a failed
  candidate leaves every entity intact. Before Save, the operation
  supports one-step Undo/Redo and uses existing rollback on staging
  failures. Qt6 inspector adds **Delete selected dropped items /
  XP orbs…** with explicit destructive confirmation.
- On **Save**, the native region transaction first applies any valid
  staged original entity edits, then removes selected original List
  members in memory, parses and validates the result, and only then
  creates a **complete original region file backup** before replacing
  the native region. Both inline Java Anvil/MCRegion and separate
  `entities/*.mca` storage are supported. `level.dat`, unknown
  entity NBT, lighting/heightmaps, and unrelated region files are
  not converted or rewritten.
- **Undo limitation, made explicit:** committing a List contraction
  shifts the persisted original entity indices, so all pre-save
  session Undo/Redo history is invalidated after a successful Save
  involving this structural action. After Save, use **File →
  Restore native region backup…** for recovery. That restoration also
  creates a fresh inverse backup. Other nonstructural operations retain
  their existing prior Undo/Redo semantics.
- `tests/test_entity_delete.py` adds source regression specifications
  for identical surviving opaque entity bytes, type preservation of
  empty Lists, modern and legacy layouts, malformed/linked/modded
  refusal, original-index stability, all-or-nothing multi-delete
  preflight, single-step staged Undo/Redo, mixed Pos edits with
  deletion, original-region snapshots, saved reopen and inverse
  region-backup restoration.

**Not yet ported:** arbitrary entity creation; deletion of general
living entities, passengers, vehicles or linked graph members;
full modded identity/link schemas; cross-chunk and cross-region
entity relocation. Original-world fixtures/CI/Qt/Mesa tests and
Windows packaging are still deferred per migration-first priorities.
Only the existing Draft PR #1 on `python3-modern-editor` was changed.

## 0.3.0a42 — Original same-chunk entity duplication with independent UUID

Extended the experimental Python 3/Qt6 native Java entity workflow beyond
deletion and simple scalar/Position/Rotation edits with **duplicate original
standalone dropped-item or XP-orb entity into the same storage chunk**.

- Added `entity_clone.py` with an intentionally narrow allowlist:
  original `minecraft:item`, `minecraft:experience_orb`, legacy
  `Item` and `XPOrb` only. Source must be in the original target
  chunk, have explicit existing UUID encoded as modern
  `UUID` IntArray[4] or legacy `UUIDMost` plus `UUIDLeast` Long
  scalars, and contain none of the known passenger, owner, leash,
  thrower or other relationship tags. Unsupported or ambiguous UUID
  layouts, missing UUID, custom entity types, existing pending NBT
  edits on the source, and cross-chunk clones are refused.
- Every cloned entity receives a newly generated UUIDv4, encoded in
  the **same native UUID field layout** the original entity uses.
  Native Pos changes only the existing three Double bytes. Every other
  original NBT field and packed member—including a dropped item's
  entire original opaque modded ItemStack Compound—retains identical
  source bytes. No item registry lookups, conversion, enchantment
  normalization or assumptions about nonstandard modded references.
  As with all original native edits, this is for offline disposable
  world copies, not a guarantee of mod-specific semantic safety.
- New strict `patch_entity_additions` writer appends staged entity
  Compound payloads to the original `Entities` list, without changing
  surviving list members or other chunk NBT. It checks source layout,
  target chunk and duplicate UUIDs among understood original identities
  and pending additions before any region replacement. It supports
  both modern separate entity region files and legacy inline Java
  Anvil/MCRegion entity Lists; no additional region file creation.
- `NativeAnvilWorld` stages new cloned payloads keyed by newly
  generated UUID, **not old list index**. The browser marks them as
  `[new, unsaved]` (negative staging index). They cannot be selected
  as further clone/edit sources until the saved file is reloaded,
  avoiding premature dependency on a list index that does not exist
  in the original file. Other original member indices remain stable.
- `EditSession.clone_entity_in_chunk` integrates original entity
  duplication into the current one-action Undo/Redo machinery.
  Undo drops the pending clone, redo reinstates its **same new UUID
  and unchanged clone bytes**. On Save, native region files are
  fingerprinted, session-locked, entirely prevalidated and fully
  backed up before atomic replacement; only then is the old
  index-based Undo/Redo history cleared. Committed entity additions
  are recoverable from **File → Restore native region backup…**.
- New source-level `tests/test_entity_clone.py` covers original
  numeric UUID encodings, opaque ItemStack/tag preservation, native
  List append integrity, unsafe/no UUID/linked/modded rejection,
  pre-save Undo/Redo and UUID stability, separate `entities/*.mca`
  saved-region backup/reopen, clone-and-original-deletion and old
  MCRegion `Level/Entities` cloning.

**Still missing:** arbitrary registered entity creation, general
mob/vehicle duplication, linked passenger/leash graph updates,
cross-chunk relocation, custom entity identity formats, full
source-component/version-aware render behavior and game-side
verification. The new tests are source specifications and have
**not yet been executed** in full CI/Mesa/Windows/game-world fixtures.
No new branch or PR; `master` remains unchanged.

## 0.3.0a43 — Strict original UUID-preserving cross-chunk item/orb transfer

Added the first experimental **native Java cross-chunk and cross-region
entity move** for the same narrow standalone dropped-item/XP-orb subset as
the preceding original entity clone/delete features. This operation MOVES an
existing entity: it retains its original UUID, instead of creating a new one.

- New `entity_transfer.py` preflights the original existing entity
  with the guarded item/orb and reference-tag allowlist, validates a finite
  destination in a **different chunk**, rewrites **only the original three
  Pos TAG_Double values**, and compares every other byte in the Compound
  to prove original UUID, modded ItemStack and unknown NBT were unchanged.
  Negative chunk coordinates use Java-compatible floor division.
- `NativeAnvilWorld.prepare_entity_transfer` requires an existing
  original source and a **pre-existing destination chunk in the SAME
  original entity storage family**: inline Anvil/MCRegion chunk entities
  stay inline; separate `entities/*.mca` entities stay separate.
  It refuses missing chunks, pending edits/deletions of the source,
  recognized UUID collisions in the destination or its staged additions,
  linked/modded/unrecognized entity identities, and over 128 simultaneous
  pending transfers. It never generates target region files or chunks.
- Staged transfers live in a dedicated original-source-index map, not
  in the same table as additions or scalar/Pos edits. While unsaved, the
  source disappears from its current Qt6 entity listing and the target
  preview shows a staged member with no original List index.
  The preview does not alter the files or renumber unrelated original
  list members. New staged items cannot be edited before saving/reopening.
- A successful native Save places both the **original source removal**
  and **destination append with the SAME original UUID** in the existing
  multi-region preflight/backup/replace workflow. Both source and target
  native region files are included in the preflight and complete backup
  when the move crosses region boundaries; original lighting,
  heightmaps and unrelated NBT bytes remain unmodified. Multi-file
  replacement offers best-effort rollback on ordinary write errors;
  it is **not crash-atomic**.
- `EditSession.transfer_entity_to_chunk` and Qt6
  **Move dropped item / XP orb to another existing chunk…** provide an
  explicit XYZ destination dialog, refusal messages and one-operation
  Undo/Redo **before Save**. A successful structural List save clears
  the pre-save Undo/Redo history due to index renumbering. After Save,
  recover via `File → Restore native region backup…`, which creates
  another safety backup.
- Source-only `tests/test_entity_transfer.py` specifications cover
  preserved original UUID and opaque custom ItemStack, negative
  coordinates, unsafe/same-chunk failures, absent target rejection,
  stable source/destination previews, pre-save undo/redo, other
  entity edits in the source, same-region multi-chunk save, and
  two separate original region-file backup coverage.

**Not implemented:** cross-dimension transfers; creating nonexistent
chunks/regions; cross-storage-family migration; arbitrary mobs/vehicles;
passengers, leashes, owners and graph references; proven global UUID
collision-free scans across entire worlds; mod-specific linked identities.
No new CI/Minecraft-version fixture tests or Windows builds have been
executed. The existing Draft PR and branch are preserved.

## 0.3.0a44 — Preflighted group cross-chunk entity transfer

Building on the original Java item/orb transfer added in a43, the
new Qt6 inspector can transfer **multiple selected original standalone
dropped items and experience orbs** with a shared XYZ offset, including
destination chunks in different existing native regions.

- `NativeAnvilWorld.prepare_entity_transfer_batch` fully preflights
  every member without mutating staged data. It verifies each original
  storage source and index, same standalone whitelist, existing source
  and destination terrain/entity chunks, absence of pending
  conflicting Pos/scalar/Rotation/deletion edits, and recognized UUID
  collisions with both destination original and staged entries.
  The batch additionally rejects duplicate source selections and
  duplicate UUIDs across selected entities, accounts for other pending
  additions/transfers against each destination List length, and limits
  the aggregate pending transfer count to 128. If any candidate is
  rejected, the entire batch is refused without changing staged NBT.
- `EditSession.transfer_entities_to_chunks` uses one preflight result
  to stage all original source-removal + target-append pairs as **one
  Undo/Redo history entry**, using existing `_apply` rollback when
  staging unexpectedly fails. The previous single-entity API remains.
  Undo/Redo is valid before Save; after an original entity List
  structural Save, old index-based history is discarded as in a43.
- Qt6 `EntityListDialog` now includes **Transfer selected items /
  XP orbs by offset…** for extended/multiple selection. Enter one
  X/Y/Z offset; every computed destination must be in a DIFFERENT
  pre-existing native Java chunk. It shows explicit multi-region
  backup and non-crash-atomic persistence warnings, never touches
  the live world until Save, and refreshes staged source members.
- All source UUIDs remain unchanged; only the existing original Pos
  TAG_Double values are patched, and each original unknown/modded
  ItemStack, other entity tag and unrelated block entity stays in
  the native original encoding. Save first preflights all touched
  original regions; the original complete source/target region files
  are backed up before replacement with best-effort rollback if a
  normal write fails.
- `tests/test_entity_batch_transfer.py` adds **source-only**
  coverage for two simultaneous transfers into one destination,
  single-step Undo/Redo and same-UUID redo, byte-for-byte original
  ItemStack/UUID preservation, missing destination rollback with
  no partial staging, same-chunk and duplicate-selection refusal,
  duplicate selected UUID refusal, bounded batch size, transfers
  into different already-existing region files, save/reopen and
  complete original region backup restoration.

**Still not supported:** arbitrary living/mob/vehicle entities,
entity group passenger/leash/owner UUID graph migration,
creating missing chunks or region files, cross-dimension moves,
full modded entity schema discovery, global identity reference
analysis, automatic registry-driven ItemStack creation or complete
old MCEdit2 UI parity. These new regression cases are committed as
source; the full real-world/Qt/Mesa/CI and Windows packaging checks
have deliberately not been run.

## 0.3.0a45 — Repeated original-state CloneTool tiling

Ported an additional original MCEdit2 Clone Tool workflow into the
Python 3/Qt6 editor: **repeat clone/tiling by XYZ step**, available from
the Edit menu and the dockable Tool Palette with a persistent
repeat-count control.

- `EditSession.clone_box_repeated(box, step, repetitions)` accepts
  1–128 repeats, constrained by the session's existing total-block
  safety limit **before loading source data**. The source selection is
  snapshotted only once and copied at `source + n × (dx,dy,dz)`
  for n=1..repetitions, retaining the original BlockState (including
  version-specific state properties / legacy numeric ID+metadata).
- When stamps overlap the source or other stamped destinations,
  source values **never cascade**. Later stamps overwrite earlier
  stamps, and deduplicated target coordinates produce **exactly one
  before/after Undo entry per changed block**, so Undo/Redo can
  restore original overwritten source and destination contents.
- The operation is explicitly **block-only**, just like the existing
  conservative offset Clone tool. All original source/target Block
  Entity coordinates are checked before mutation; the backend
  preflights every changed target against existing native Java
  sections, unsupported IDs and protected positions before any
  staging. Entity NBT, unknown/modded BE payloads, biome data,
  lighting/heightmaps and untouched native chunk data are not
  transformed. Missing original chunks are not generated.
- Edit menu `Repeat clone / tile selection…` prompts for XYZ step
  and repeat count. The tool dock adds a repeat/tiling choice and
  persistent count (1–128); repeat executes via the same guarded
  session transaction. A rejected step, count, source BE, destination
  BE, invalid ID or missing section creates no staged blocks or Undo
  record.
- `tests/test_clone_tiling.py` includes source-only specifications
  for disjoint and overlapping stamps, last-writer-wins with original
  snapshot, single grouped Undo/Redo, oversized/boolean count and
  zero/fractional-offset refusal, source/target BE fail-closed
  protection, late native section and state preflight, exact legacy
  ID/metadata retention, modern unknown mod BlockState retention,
  both native Java Anvil and MCRegion original NBT byte preservation,
  full-region backup, save/reopen and post-save Undo.

**Remaining original Clone Tool gaps:** independent rotation/scaling
preview, visual ghost-placement/drag manipulation, per-copy palette or
entity/Block Entity aware tiling, complex modded orientation/identity
graph updates and large-volume streaming of edits. No full
Minecraft-version/Qt/Mesa test suite or Windows packaging run has
been executed for this migration-first iteration; the source test
cases have not been claimed to pass.

## 0.3.0a46 — Rotated / mirrored original CloneTool copies

Expanded the Qt6 Clone Tool beyond repeat/tiling with **oriented
copies** of an original selection: clockwise 90°, counterclockwise
90°, mirror East/West (X), mirror North/South (Z). Any orientation can
be combined with 1–128 copies at multiples of one XYZ step, and is
available in both the Edit menu and dockable Tool Palette.

- New `EditSession.clone_box_transformed(box, offset, operation,
  repetitions)` takes a **single immutable snapshot** of the original
  selection before any staging, computes the new rotated rectangle
  dimensions and transformed per-block coordinates, then applies
  verified native horizontal orientation rules using the existing
  `transforms.transform_state` and `transform_position` codecs.
  The source is left untouched. Repeated copies may overlap each
  other or the original selection; the **last planned stamp wins**
  and every unique changed coordinate appears once in history.
- Uses the strict established vanilla `BlockState` orientation
  allowlist (stairs, rails, directional connections, axis, facing,
  door hinges, standing sign rotation, etc.) and the explicitly
  supported pre-flattening numeric ID/metadata rules (stairs, logs,
  torches, known invariant IDs). Unknown modded blocks, unverified
  metadata and orientation-related properties are **refused**,
  rather than guessing or normalizing their NBT.
- BE/entity cloning is **not** implied. Every source and destination
  block entity position is checked before staging, and the native
  backend preflights all actual changed targets for writable
  original sections, protected BEs and valid original block states.
  The planned stamp count is checked against `EditSession.max_blocks`
  before reading source blocks. Invalid operation, zero step,
  non-integer/over-limit repeats, unknown oriented blocks, missing
  sections or BE conflicts produce no committed edit or Undo record.
- The Edit menu provides the XYZ step, orientation choice and copy
  count through Qt6 dialogs. The persistent Tool Palette adds a
  rotation/mirror combobox next to the repeat-count spinner and
  dispatches directly to the same native session operation.
  QAction triggered(bool) is explicitly isolated from optional
  menu parameters so the menus cannot accidentally interpret a
  signal Boolean as the copy count/operation.
- `tests/test_clone_transformed.py` adds **source-only regression
  specifications** for rotated asymmetric footprints, block-state
  facing/hinge retention, mirror and repeated-overlap behavior,
  single-step Undo/Redo, legacy ID/metadata fidelity,
  unknown-orientation, source/destination BE and late-section
  refusal, count and offset limits, original Anvil modded BE/other
  tag byte preservation, old MCRegion entity/lighting tag
  preservation, full-region backup and save/reopen.

**Not yet migrated:** arbitrary-angle rotations, scaling, visual
ghost previews and drag handles, broad modded orientation/structure
schema support, automatic BE/entity/ownership graph-aware copying,
large streaming clones and the complete original MCEdit2 Clone Tool
UI parity. No full live-Minecraft/Qt/Mesa CI matrix or Windows build
has been executed in this migration-first round.

## 0.3.0a47 — Native Clone Tool integer XYZ scaling and geometry preview

Ported an additional original MCEdit2 Clone Tool workflow to Python 3/Qt6:
**independent integer nearest-neighbour scaling** of an original native
block-only selection, optionally repeated as a row of XYZ-offset stamps.
The scale factors are 1–8 on each X/Y/Z axis. Downscaling, fractional
interpolation, scaling modded Block Entity contents and arbitrary-angle
geometry deformation remain explicitly unsupported.

- New `clone_scaling.py` defines a *pure read-only*
  `plan_scaled_clone` with output dimensions, first-copy and
  all-copy original-coordinate bounds and the **planned, not merely
  unique, block-write count**. The planner rejects non-integer/
  Boolean/zero/over-8 axis factors, repeat counts outside 1–128,
  zero or non-integer XYZ steps, over-limit operations and
  coordinates beyond native Java X/Z bounds **before opening any
  original region or reading world blocks**.
- `EditSession.clone_box_scaled` takes a **single immutable
  original selection snapshot** and expands each original native
  `BlockState` by nearest neighbour across the independent
  X/Y/Z scale factors. The unchanged block-state payload is copied
  even for opaque modded IDs/state properties; no guessed modded
  rotation, item registry lookup, palette normalization or NBT
  conversion is performed. Each repeated stamp starts at
  `selection.minimum + repeat × XYZ_step`. The source remains in
  place; later stamps win when expanded footprints overlap.
  Destination coordinates are deduplicated before the grouped
  native Undo/Redo transaction, preserving one original-before
  record for each changed block even on source/destination overlap.
- The operation remains **block-only**. Every source and potential
  target Block Entity position is checked before any mutation;
  the existing native backend preflights all actual changed target
  blocks for original writable sections, supported ID encoding
  and protected BE coordinates. Missing destination sections
  are not generated. Entity NBT, other original Block Entities,
  world heightmaps/lighting and chunk metadata are never scaled.
- Qt6 Edit menu adds **Clone selection with XYZ scaling…**. The
  new dialog accepts independent factors and copy count, renders a
  live geometry-only bounds/write-budget preview, and disables OK
  for invalid/over-limit plans. This is explicitly *not* a 3D
  ghost preview or a guarantee that all target sections/BEs are
  writable; actual edits still perform the full original native
  preflight. The dockable Tool Palette also stores independent
  X/Y/Z scale factors and uses its existing repeat-count spinner.
  Menu actions isolate QAction's `triggered(bool)` signal from
  optional operation arguments.
- `tests/test_clone_scaling.py` adds **source-only regression
  specifications** for exact output envelopes, negative offset
  geometry, independent scaling axes including vertical Y,
  original-snapshot noncascading overlap, grouped Undo/Redo,
  malformed/large-factor or repeat rejection, deliberate planned
  work limits even with heavily overlapping destinations,
  source/target BE and late native-section refusal without
  partial mutation, unchanged modded Anvil block state and
  original BE/other-tag bytes, and old MCRegion numeric ID/data
  and lighting/entity/unknown-tag bytes through native backups,
  save/reopen and post-save Undo.

**Remaining:** fractional or shrinking resampling, visual 3D ghost
clone, dynamic drag/rotate/scale handles, registry-correct modded
block orientation/BE identity graphs, full original Clone Tool
interactions and all real-world version test fixtures. No full CI,
Minecraft reload, Qt/Mesa execution or Windows package build has
been run for this migration-first round.

## 0.3.0a48 — Read-only 3D Clone Tool target wireframe placement preview

Ported the first interactive original MCEdit2 Clone Tool **3D placement
preview** to the Python 3 / Qt6 editor. The new feature deliberately
visualizes *bounded destination volumes* as cyan/green wireframes,
not simulated textured voxel meshes or an assurance that placement is
writable. Source NBT and game region files are never read or mutated by
the preview geometry planner.

- Added pure-Python `clone_preview.py` containing
  `plan_clone_ghost` and `ghost_wireframe_vertices`. It accepts four
  existing native Clone Tool actions: one offset copy, repeated
  XYZ-step tiling, clockwise/counterclockwise 90° or X/Z-mirrored
  repeat clones, and nearest-neighbour integer XYZ scaled repeat
  clones. For each 1–128 copy it computes the exact original-world
  *half-open* target volume, transformed footprint, maximum planned
  writes and Java horizontal-coordinate range. It reuses the a47
  pure integer scale planner and established strict horizontal
  transform dimensions, without reading block data, querying
  modded registries or generating any chunk.
- The viewport's existing 3D GL_LINES path now renders clone
  wireframe boxes. Earlier copies have cyan outlines; the final
  copy is green. The classic fixed-radius OpenGL renderer merges
  the overlay into its existing selection/gizmo line geometry.
  The default **streamed OpenGL** renderer draws those same
  outlines using its 9-float shader vertex layout even when no
  ordinary selection outline is active. Camera orbit/zoom continues
  to work; showing/hiding previews does **not** invalidate or
  rebuild decoded chunk geometry.
- The dockable Qt6 Tool Palette now offers a persistent
  **3D Clone Tool placement** control with four modes, X/Y/Z
  copy-step coordinates, existing repeat count, rotation/mirror
  and scale factors, plus explicit **Preview in 3D**, **Apply
  preview** and **Clear** buttons. A preview automatically
  switches to 3D when available, reports outlined target counts,
  planned cells and copy footprints, and never stages world edits.
  Changing an input, the current selected region, world, dimension
  or view mode invalidates the preview so obsolete coordinates
  are not silently used.
- Applying a preview rechecks the **same world session and exact
  original selection**, then delegates to the corresponding
  existing native `EditSession.clone_box`,
  `clone_box_repeated`, `clone_box_transformed`, or
  `clone_box_scaled`. These native paths recompute source
  states and run full original Block Entity, target section,
  original numeric-ID/metadata and orientation preflight
  before staging. A rejected native operation does not clear
  the preview or present the write as successful; a successful
  application clears all preview boxes and creates the existing
  single grouped Undo/Redo operation. The geometry-only view
  never claims to have checked collisions, BE links or modded
  orientation properties.
- Added `tests/test_clone_preview.py` pure geometry source
  regressions for each mode, scaled and asymmetric rotated
  target extents, negative offsets, limits, line vertex format,
  malformed boxes and no file operations. Added
  `tests/test_clone_preview_qt.py` offscreen controller/source
  tests using a synthetic 3D view, specifying **no writes
  before Apply**, preservation of grouped Undo, parameter and
  selection invalidation, BE refusal without committing
  the preview, and scaled mode geometry/application matching.

**Still not ported:** transparent textured per-block ghost meshes,
3D mouse-drag destination handles and automatic placement snapping,
arbitrary/fractional scale, multi-block BE/owner/UUID reference graph
transfer and complete original MCEdit2 Clone Tool UI parity.
The preview is deliberately *read-only* until Apply. All new tests
are committed as source specifications but have not been executed
in a complete real-world/Qt/Mesa/CI/Windows matrix.

## 0.3.0a49 — Mouse-draggable 3D Clone Tool placement and grid snapping

Extended the a48 read-only cyan/green Clone Tool target-volume preview
with **interactive mouse-ray placement** in both the base and streamed
OpenGL views. Preview movement changes only the planned original XYZ
clone step; it never edits source chunks, player NBT, Block Entities
or the original disk region file until the user explicitly presses
Apply and full native preflight succeeds.

- The first wireframe now includes a visible white three-axis drag
  cross. Hold **Ctrl+Alt+left mouse** on that center to drag the
  preview horizontally in the original world X/Z plane, or use
  **Ctrl+Alt+Shift+left mouse** for a vertical Y-axis drag.
  The viewport reuses its existing camera-ray/world-plane and
  axis-to-ray math and rejects misses rather than modifying
  the current selection or painting a block. Each gesture
  captures its original ray origin and handle center to avoid
  feedback or drift as preview coordinates move under the cursor.
- Pure `snapped_clone_drag_offset` validates integer XYZ
  displacements and snaps the *first* destination minimum to
  an **absolute Java world-coordinate grid** of 1, 2, 4, 8 or
  16 blocks, including negative coordinates. Horizontal dragging
  preserves Y; Shift-vertical dragging preserves X/Z. The
  geometry planner rejects out-of-bounds/over-budget targets
  without changing the previous valid preview.
- The Qt6 Tool Palette adds a persistent grid-size selector and
  documents the input gestures. Dragging updates the exact
  preview XYZ step spin controls with signal blocking, so those
  controls cannot accidentally dismiss the preview midway
  through an in-progress gesture. Both OpenGL paths redraw the
  shared wireframe without rebuilding native section meshes.
  On unsuccessful plans or unrepresentable XYZ step settings,
  no partial UI/preview mutation is committed.
- Existing repeated clones use `source.min + n × step`.
  Accordingly, **dragging the first preview changes the repeat
  step**: later copies relocate according to their respective
  multiples, rather than translating the full group rigidly.
  This behavior is stated in the UI. A later migration may
  separate group origin from repeat stride, alongside textured
  per-block ghost rendering.
- Apply continues to verify exact original source selection and
  world session and to execute the appropriate existing
  block-only native Clone Tool operation with current world
  Block Entity and target section preflight. Save/region-backup
  and single-step Undo/Redo behavior are unchanged.
- `tests/test_clone_drag.py` adds source regressions for
  absolute/negative-coordinate snapping, repeat geometry,
  malformed/Boolean factors, fixed GPU 8-float wireframe
  stride and white drag control, offscreen camera-ray handle
  picking, Qt controller coordinate synchronization,
  no world writes until Apply, vertical dragging,
  selection invalidation, and out-of-world movement refusal.

**Still missing:** actual semitransparent textured/opaque per-block
ghost models, physical drag/drop collision checking, rigid-group
placement origin separate from clone repeat step, arbitrary rotations,
fractional/downscaling, full modded BE/entity identity graph copying
and full original Clone Tool UI parity. The new test cases were
committed as source without executing the full GL/Mesa/Qt,
real-world Minecraft-version, CI or Windows package test matrix.

## 0.3.0a50 — Rigid-group Clone Tool translation independent of repeat stride

Completed a key original Clone Tool placement behavior in the native
Python 3/Qt6 editor: **whole-group position and per-copy spacing are
now independent**. Previously dragging the first destination
rewrote the XYZ repeat step and moved subsequent stamps by different
multiples. As of a50 the 3D drag translates the entire group,
preserving relative spacing, orientation and scaling.

- The immutable read-only `CloneGhost` plan now contains two
  separate three-integer vectors: `offset` (per-copy original XYZ
  stride) and `placement` (one fixed group translation). For
  repeat index n, each target starts at
  `source.minimum + placement + n × offset`.
  This same formula is implemented in the pure scaled geometry
  planner, repeated native clones, strict rotated/mirrored native
  clones and integer-scaled native clones. The one-copy offset
  clone uses the sum of both vectors on Apply. All old call sites
  remain behavior-compatible because placement defaults to zero.
- The Qt6 Tool Palette now has independently editable
  **Step X/Y/Z** and **Group position X/Y/Z** controls. The existing
  Ctrl+Alt left mouse white-handle gesture and Shift vertical drag
  now change only the group position; the original repeat stride
  remains unchanged. The first destination minimum still snaps to
  a selectable **absolute original-world grid** of 1, 2, 4, 8,
  or 16 blocks, including negative world coordinates. A rejected
  target leaves all previously valid preview geometry and spin-box
  values intact. The preview summary displays both group position
  and step to avoid any ambiguity.
- `snapped_clone_drag_placement` is a pure helper built on the
  original verified grid math. It validates all integer vectors
  before computing new positions, translates the first copy,
  and derives an updated rigid-group vector while leaving the
  step untouched. Both OpenGL paths reuse the existing bounded
  cyan/green wireframe and white drag control; GPU chunk meshes
  are not rewritten simply to reposition a preview.
- The actual **Apply preview** route uses the *same group placement
  vector* that produced the wireframe. Existing originals are
  still snapshotted once, transformed only by the verified
  native orientation codec, or duplicated without interpreting
  opaque/modded original BlockStates. Original source/target
  Block Entity checks, native sections, ID/metadata guards,
  region backup and single-step Undo/Redo remain in effect.
  A failed Apply must leave the original save unchanged.
- `tests/test_clone_group_placement.py` supplies source-level
  regression specifications for rigid spacing, transformed and
  scaled output envelopes, incorrect/out-of-bounds placement,
  snapshot and overlap undo semantics, opaque modded state copies,
  rejected destination Block Entities, and original Java Anvil
  region/unknown NBT byte preservation. Additional
  `tests/test_clone_group_placement_qt.py` cases specify mouse
  movement vs editable XYZ step, manual group position, scaled
  Apply, invalidation, and no native changes before Apply.
  Updated the previous drag and GL white-cross vertex source
  expectations for the current behavior.

**Not yet implemented:** per-block translucent textured voxel
ghosts, automatic native chunk/Block Entity collision markers,
freeform or fractional scaling/rotation, complex linked BE/entity
graphs, and complete original MCEdit2 GUI/tool parity. The added
source tests have deliberately not been executed against a full
Qt/Mesa/Minecraft-version matrix or Windows packaging workflow.

## 0.3.0a51 — Bounded read-only translucent Clone Tool material preview

Expanded the Qt6 Clone Tool beyond a wireframe-only 3D placement UI.
For plans of **up to 2,048 planned original block cells**, the
preview now produces a translucent render of the original BlockState
snapshot in both the classic and streaming OpenGL 3D views, alongside
the existing per-copy wireframe bounds and white drag handle.

- Added `clone_ghost_voxels.py`: snapshot original BlockStates
  **once** from the selected source volume, refusing any source
  Block Entity before representing it as an ordinary block-only
  material ghost. Compute projected original positions from the
  exact same immutable `CloneGhost` group-position/step formula
  used by native Apply. Supports single offset, repeated, 90°
  rotated/mirrored, and integer XYZ-scaled repeat modes.
  The established conservative `transform_state` codec is
  reused for every oriented voxel. Unknown oriented/modded
  states refuse the voxel visualization rather than claiming
  an invented direction. Later overlapping copies win, and
  original air states erase preceding preview voxels before
  filtering invisible geometry. None of this mutates the
  editor session or region file.
- The bounded GPU mesh is a simplified **full-cube preview**:
  atlas-backed cube faces where textures exist, otherwise
  deterministic native BlockState colors. The streaming
  renderer uses its **existing stable atlas** and compatible
  9-float GPU vertex layout, while refusing to draw guessed
  cubes for resolved non-full-cube JSON models and fluids;
  those shapes remain covered by visible wireframe bounds.
  The classic renderer uses its original 8-float full-cube
  texture path. Complex multipart models, fluid surfaces and
  unrecognized modded render geometry are **not** represented
  as accurate per-element meshes.
- The translucent material is drawn in a separate bounded
  alpha-blended pass (opacity 0.42) after world solids with
  depth writes off. Both renderers retain the original
  red/cyan/green selection and Clone Tool line overlays.
  The streaming viewer caches immutable ghost geometry for
  camera movement and uses no SectionStreamer edits,
  background write jobs, modified NBT or chunk remeshing.
  Resource-pack atlas replacement invalidates ghost UV
  caches, and closing world sessions clears original
  preview geometry and GPU counts.
- The original source states are read **at most once when
  creating the preview**; mouse dragging, absolute grid snap,
  independent group translation and new wireframe projection
  reuse the same cached native snapshot, requiring no
  repeated disk or BlockState scans. Parameter/selection
  changes invalidate the preview. The **Apply** action
  still rereads the current original world and independently
  validates all source/target BEs, valid original region
  sections, legacy metadata, allowed orientation rules and
  native-save constraints before staging a single grouped
  Undo/Redo operation. A material preview is never approval
  for a potentially unsafe world edit.
- For selections over the 2,048 planned-cell visual limit,
  unreadable original blocks, source BEs, or unsupported
  orientation, the editor retains the bounded **wireframe-only**
  target preview and explains the fallback in the Qt6 Tool
  Palette and status bar. Large previews do not secretly
  allocate oversized opaque region/atlas snapshots.
- Added `tests/test_clone_ghost_voxels.py` source regressions
  for exact original-state projection in every mode,
  source caching and no disk writes, projected air/overlap
  precedence, strict BE/orientation refusal, source/target
  limits and original streaming atlas tile/mesh layout.
  Added `tests/test_clone_ghost_qt.py` offscreen Qt
  controller specifications for material previews, dragging
  without new world reads, source BE fallback, oversized
  wireframe fallback, clear/parameter-change cleanup and
  single-step Undo/Redo. Updated the existing fake GL
  viewport to support independent material ghost state.

**Remaining:** fully accurate semitransparent block JSON element
rendering for stairs, slabs, models and animated fluids; material
preview for very large selections; sorted per-face alpha for
intersecting translucent surfaces; linked/owned BE and entity
ghosts; full Windows and Minecraft-version integration tests.
The new tests were **committed as source only**, not executed
against the full Qt/Mesa/real-world CI matrix.

## 0.3.0a52 — Complete right-angle Clone Tool horizontal rotation

Added distinct original-format **180°** and **clockwise 270°**
rotation choices to the existing Clone Tool and ordinary selection
rotation UI. Prior clockwise/counterclockwise 90° and X/Z mirror
behavior stays intact. Both new angles share the existing 3D Clone
Tool read-only target wireframe and bounded BlockState material ghost
logic; they are not new file formats or new rotation engines.

- `transforms.py` now explicitly maps `half_turn` and
  `clockwise_270` for original block-direction properties, corner
  rails and ascending rails, vanilla 16-step standing sign rotation,
  X/Z horizontal axis, side-connection properties, and strict native
  legacy stairs/torches/log metadata. Half turns preserve X/Z box
  dimensions and swap both opposite corners; clockwise 270° exchanges
  X/Z extents and agrees with counterclockwise 90°.
- Existing `EditSession.clone_box_transformed` and
  `transform_selection` reuse the strict new mappings, fully
  preparing all transformed coordinates and BlockStates before any
  edit and preflighting protected Block Entities and native target
  sections. Unsupported modded/unknown original orientation data
  fails closed for the whole operation. Original source data is
  preserved for offset clones; repeated clones use the same
  immutable original snapshot and one undo group. In-place
  selection transforms retain undo history as before.
- Qt6 Edit menu, oriented Clone Tool dialog and Tool Palette
  orientation dropdown expose both new angles. Original
  selection 180° and 270° actions also appear in the menu/palette.
  `plan_clone_ghost` and `project_clone_ghost` automatically use
  the new transform geometry for existing drag/snap previews and
  2048-cell bounded preview rules.
- `tests/test_clone_half_turn.py` adds source-only regressions for
  rectangular geometry, 180° roundtrip, clockwise 270° equivalence,
  stairs, rails, 16-step rotation, redstone side connections, log
  axes, strict legacy numeric metadata, unknown modded refusal,
  offset/in-place copy and Undo/Redo, source/target BE rejection,
  3D ghost coordinate agreement, and original Anvil/MCRegion
  unaffected block entity/entity/light/unknown NBT/region backup.

**Remaining:** non-right-angle arbitrary rotations, registration
of per-mod state transforms, entity and Block Entity graph-aware
rotation, model-exact translucent previews, downscaling and full
original Clone Tool UI interaction parity. Full CI, Qt/Mesa/Windows
packaging and live Minecraft-version testing were intentionally
deferred; newly written regression source is not asserted to pass.

## 0.3.0a53 — Clone Tool bounded integer XYZ downscaling

Added the missing **nearest-neighbour integer reduction** path to the
existing Python 3 / Qt6 Clone Tool. The user may divide X, Y and Z by
independent 1–8 integer factors, repeat the result with a fixed XYZ
stride, and preview/edit its rigid 3D placement using the existing
non-destructive wireframe/BlockState ghost system.

- `clone_scaling.plan_reduced_clone` calculates exact ceil-divided
  output dimensions and world-coordinate extents, checks 1–128 copy
  counts, nonzero integer XYZ step, placement, Java horizontal bounds,
  original source volume and planned target writes against the
  native session limit before any source NBT reads. A source axis of
  length 5 reduced by 2 becomes 3 cells (source positions 0, 2, 4);
  the trailing partial group is not dropped.
- `EditSession.clone_box_reduced` **checks every original source cell
  for Block Entities, even unsampled cells**, then snapshots only the
  first original block of each XYZ sampling group. This makes
  approximate resizing deliberate while never silently discarding a
  protected BE. It copies each original native BlockState unchanged;
  no item/entity NBT, ownership, modded registry or orientation
  guessing. Every destination is checked for protected BEs and
  writable native sections/IDs before any staged mutation.
- Output copies are stamped from one immutable source sample; later
  copies overwrite earlier duplicates. Each unique changed
  destination is recorded only once in one undoable transaction.
  Undo/Redo remains available after original region saves, unlike
  structural entity List edits. Full native region backups apply as
  with other block edits.
- Qt6 **Downscale clone by integer XYZ divisors…** action is available
  in the Edit menu and dockable Tool Palette with separate downscale
  factor controls and a read-only geometry/cell-budget preview.
  Its `reduced` 3D placement mode supports original group
  dragging/grid snap/stride, complete footprint wireframes and
  at most 2048 original BlockState material-ghost cells; oversized
  or BE-protected previews fall back to wireframes only.
  Apply always reruns actual original native preflight.
- `tests/test_clone_reduction.py` specifies exact ceil bounds,
  negative stride and group placement, type/range/cell-limit
  rejection, trailing partial groups, 3D XYZ sampling, overlap
  with frozen source, single-step Undo/Redo, unsampled-source-BE
  refusal, late-target-BE/native-section rejection, ghost/wireframe
  parity, and Anvil/MCRegion save/reopen + backup preservation of
  original unrelated Block Entity/Entities/lighting/unknown NBT.

**Limitations:** This is intentionally lossy nearest-neighbour
**block-state-only reduction**, not a semantic downsizing of machines,
inventories, entities, links or modded multiblocks. No arbitrary
fractional resampling, rotation combined with scaling, model-accurate
transparent ghost for all Minecraft blocks, or live-world compatibility
guarantee. The regression tests are committed as source but the full
version-matrix/Qt/Mesa/Windows builds and game-world checks remain
deferred while migration work continues.

## 0.3.0a54 — Combined orientation and integer XYZ scaling

Completed the missing combined original-format Clone Tool operation:
**rotate/mirror first, then enlarge the rotated output by independent
integer X/Y/Z scale factors 1–8**, with 1–128 repeated copies and
independent rigid-group placement.

- `plan_oriented_scaled_clone` uses the original strict horizontal
  right-angle/mirror geometry, with 90° and 270° swapping the X/Z
  footprint *before* scale is applied to destination-world axes.
  The existing bounds, original-region coordinate, planned-cell,
  destination-size and copy-count restrictions all apply before
  reading any original block state.
- `EditSession.clone_box_oriented_scaled` snapshots the entire
  original selection and applies the verified original-version
  `transform_state` mapping before nearest-neighbour replication.
  Unknown modded orientation schemas or old legacy ID mappings fail
  closed. Every source/target Block Entity location and changed
  native section/ID is checked before writing anything. The operation
  copies block states only: **no Block Entity, entity, UUID, link,
  inventory or modified/unknown NBT record is reconstructed**.
- Copies with overlapping target coordinates use last-stamp-wins
  semantics from a frozen source snapshot. Each unique changed
  coordinate is written once and appears once in a grouped
  Undo/Redo record, including after native original-region save.
- Qt6 adds **Clone rotated and scaled selection…** with orientation,
  independent destination XYZ expansion, repeats and a read-only
  geometry/budget preview. The dockable Clone Tool now supports
  **Rotate then scale XYZ** as a placement mode using its existing
  3D drag, world-grid snap, copy stride, rigid-group placement,
  bounded wireframe volumes and optional 2,048-cell translucent
  original BlockState preview. Material ghost projection performs
  the same orientation-then-scaling arithmetic as Save/Apply.
- Source regression specifications in
  `tests/test_clone_oriented_scaled.py` cover rectangular source
  geometry and axis order, source overlap and stable Undo/Redo,
  invalid factors/count/coordinates and refusal of unsupported
  modded orientation, BE safety and late native destination
  preflight, preview vs actual source-state projections, and
  original modern Anvil/legacy MCRegion region preservation
  including unrelated opaque Block Entity records.
- Concurrently introduced duplicate Python definitions were
  consolidated to one canonical planner, one core operation, one
  Qt6 dialog and one Apply dispatch for `oriented_scaled`.
  No new PR or branch is created.

**Not ported:** arbitrary-angle fractional rotation, general
modded orientation registries, safe semantic scaling of entities,
inventories or linked multiblock systems, photorealistic full-model
transparent ghosts, or crash-atomic multi-region transactions.
Full CI, Mesa/Qt/Windows builds and live version-matrix testing
remain deferred as requested.

## 0.3.0a55 — Guarded Up/Down (Y) mirroring in native Clone Tool

Added loss-averse **vertical reflection** as an explicit original-format
editor operation rather than pretending arbitrary Minecraft blocks have
universal pitch/roll metadata conversions. It is supported both by the
in-place Qt6 selection transform and by the existing Clone Tool oriented
single/repeated copy, rotate-then-scale and read-only 3D ghost planning.

- `transforms.py` now recognizes `mirror_y`. Geometry reflects the
  local selection coordinate `y → sizeY−1−y` while preserving X/Z
  coordinates and the bounding-box dimensions. Applying the operation
  twice restores the original position and supported native state.
- For modern Java vanilla blocks, only explicitly verified simple
  transformations are allowed: stairs `half=top/bottom`, slabs
  `type=top/bottom/double`, trapdoors `half=top/bottom`, original
  `axis=x/y/z` logs/wood/stems/hyphae/pillars, and verified static
  invariant blocks. Horizontal `facing`, stair `shape`, power/open
  flags, waterlogging and other whitelisted metadata remain unchanged.
  Missing/wrong-valued required fields and unknown properties fail
  closed.
- For legacy numeric Java worlds, original stair upper-half bit 4
  and the original vanilla slab upper-half bit 8 (IDs 44/126/182)
  are inverted with all other original metadata bits retained.
  Verified invariant IDs and existing log IDs retain exact metadata;
  other numeric IDs, modded blocks, modern doors/observers/pistons
  and custom/linked vertical geometries are deliberately refused.
- `EditSession.transform_selection`, `clone_box_transformed`,
  `clone_box_oriented_scaled`, the 3D Clone Tool preview plan and
  bounded original-state ghost projection all reuse the **same**
  native coordinate/state mapping. The Qt6 Edit menu, clone dialogs,
  tool dock and orientation combo now expose an explicit guarded
  `Mirror Up / Down` choice. No dedicated fake NBT conversion is used.
- All original Block Entity sources and targets are rejected before
  mutation by existing BE checks; unsupported custom block orientation
  and invalid/missing original native sections refuse the whole
  operation without a partial staged write. Existing Undo/Redo
  grouping, full-region original backups and unrelated BE/entity NBT
  preservation remain unchanged.
- Added source regression specifications in
  `tests/test_clone_vertical_mirror.py` for involutive geometry,
  correct modern slab/stair/trapdoor and legacy stair/slab metadata,
  unsupported modern/modded/linked refusal, BE and late-section
  fail-closed checks, original-state 3D ghost matching actual
  oriented clones, combined oriented+scaled copies, single-step
  Undo/Redo, and original Anvil / MCRegion save, backup and opaque
  NBT preservation.

**Not implemented:** arbitrary-angle X/Z-axis rotation, roll/pitch
of complex blocks, vertical mirroring of doors/multiblock structures,
entity/passenger graphs, modded orientation registries or unsafe
linked Block Entity NBT. Tests above are committed as **source-only**
and have not been run against the full Qt/Mesa/Minecraft-version
matrix or packaged for Windows. The existing Draft PR and branch are
retained; `master` is untouched.

## 0.3.0a56 — Native source-air-transparent Clone Tool stamps

Added a **non-destructive ignore-source-air mode** for original-format
block-only offset and repeated Clone Tool copies, preserving existing
destination terrain under skipped air cells.

- `clone_mask.is_vanilla_source_air` recognizes ONLY the three exact
  property-free vanilla Java BlockStates `minecraft:air`,
  `minecraft:cave_air`, and `minecraft:void_air`. Legacy numeric
  ID 0, metadata 0 is already normalized to `minecraft:air` by the
  native old-Anvil/MCRegion backend. Unknown modded `air`-named
  blocks, invalid legacy metadata and fake vanilla air with extra
  properties are never silently skipped.
- `EditSession.clone_box(..., skip_air=True)` and
  `clone_box_repeated(..., skip_air=True)` freeze the ORIGINAL
  selection snapshot and filter only exact recognized air positions
  out of the target write set. Repeated stamps never cascade writes
  through the source; overlapping non-air stamps preserve
  last-stamp-wins and unique before/after Undo records.
- Every original SOURCE coordinate is checked for any original Block
  Entity **even if the block appears to be air**; potentially unsafe
  source content is never quietly ignored. Only destinations that
  would ACTUALLY be written require target BE/section/ID preflight:
  a protected target underneath an omitted source-air cell is
  intentionally left untouched. All other original world NBT and
  modded BE bytes remain unchanged.
- Existing native size caps, coordinate guards, target section/ID
  validation and single-action Undo/Redo still apply. No new
  sections or chunk files are synthesized.
- Qt6 **Clone selection, ignore source air…** and **Repeat clone,
  ignore source air…** menu entries, plus a persistent Tool Palette
  checkbox for ordinary offset and repeat cloning, expose the
  masking mode without changing default clone behavior.
- `tests/test_clone_air_mask.py` adds source-only cases for
  exact vanilla versus modded air identity, masked target retention,
  original-snapshot overlap, no duplicate Undo baselines, source
  and written-target BE protection, skipped-target BE preservation,
  late invalid destination refusal, invalid option types, modern
  Anvil opaque NBT/backup/reopen and old MCRegion numeric
  metadata/unknown NBT retention.

**Scope boundary (as of a56; superseded by a57):** Air transparency at a56 was
available for *direct unrotated offset and repeated stamps only*. The existing
3D Clone Tool wireframe/ghost preview and oriented/scaled/reduced
clones do NOT yet apply this optional source-air mask. The dock
labels this distinction explicitly. This does not provide safe
Block Entity/entity transfer, custom mod identity handling,
model-exact preview, or crash-atomic multi-region persistence.
Regression specifications have been committed as source only;
full Qt/Mesa/real-world version tests and Windows packaging
remain deferred.

## 0.3.0a57 — Unified native source-air masking for all block-only Clone Tool modes

Extended the strictly opt-in a56 source-air transparency beyond direct
offset/repeated stamps to **rotated/mirrored**, integer XYZ **scaled**,
**rotated-then-scaled** and nearest-neighbour XYZ **reduced** native clones.
The 3D Clone Tool voxel ghost and its Apply action now use the same
saved mask flag.

- `EditSession.clone_box_transformed`, `clone_box_scaled`,
  `clone_box_oriented_scaled` and `clone_box_reduced` accept an optional
  strict Boolean `skip_air=False` keyword, keeping old default behaviour.
  Masked source recognition uses only exact property-free vanilla
  `minecraft:air`, `minecraft:cave_air`, `minecraft:void_air` from
  `clone_mask.is_vanilla_source_air`; modded air-named blocks and unknown
  legacy metadata are never discarded by name heuristics.
- Source BE protection still scans **EVERY** original source voxel,
  including masked air and unsampled reduction cells. A masked source
  contributes **no write target** after transformation, expansion, or
  sampling; the destination's existing state and protected BE stay
  unchanged. All actual writes retain native BE, missing-section,
  ID/metadata, bounded-work and single-step Undo/Redo preflight rules.
- The original source is read before any staged mutation, ensuring repeat
  overlap is non-cascading. Masked air does not erase non-air blocks from
  earlier stamps, and the final unique target set retains one
  before/after Undo record per point.
- `CloneGhost.skip_air` and `plan_clone_ghost(skip_air=…)` preserve
  transparency in read-only 3D plans and drag recalculations. Voxel
  ghost generation applies the mask **before** state rotation, scale or
  reduction and before last-stamp overwrite, matching native write
  geometry; target box wireframes remain full footprint bounds.
  Preview Apply passes its saved mask flag to every clone mode, including
  unrotated clones, and repeats source/target safety checks at write time.
  A preview does not certify native world writeability.
- Qt6 Tool Palette now has one **Ignore exact vanilla source air**
  checkbox for all block-only Clone Tool modes, including the 3D preview;
  changing it invalidates stale previews. Scaled, rotated-scaled and
  reduced clone dialogs also expose an explicit checkbox, with defaults
  inherited from the palette. The transformed-copy dock action uses
  the palette flag. All former menu defaults remain unmasked.
- Added `tests/test_clone_transformed_air_mask.py` source-only
  regression specifications covering rotated/scaled/oriented-scaled/
  reduced hole preservation, protected skipped-target BEs versus
  complete source BE scans, modded-air identity, strict Boolean flag
  rejection, noncascading overlap, plan/ghost fidelity, original modded
  Anvil BE/unknown NBT preservation, save/reopen and grouped Undo.

**Still missing:** linked BE/entity transfer or rotation, full modded
orientation registries, arbitrary fractional transforms, preview of
modded multi-element models and fluid faces, crash-atomic cross-region
persistence. Existing historical a56 documentation describes that
version's limitation and is superseded by this section. Full CI,
game-version/Mesa/Qt/Windows packaging tests were not run.

## 0.3.0a58 — Clone Tool destination-only-air placement

Implemented conservative **paste only into destination air** masking for
the full native block-only Clone Tool path. This is independent of the
existing **skip original source air** option, so users can combine both.

- `EditSession._guard_clone_destinations` runs *after* each clone mode has
  snapshotted its entire original source and resolved all stamp overlap,
  but *before* staging any world mutation. The check reads the current
  ORIGINAL target BlockState, and keeps a write only when its identity is
  exactly property-free vanilla `minecraft:air`, `minecraft:cave_air`,
  or `minecraft:void_air`. Existing named blocks, opaque modded
  blocks, damaged legacy numeric states, and unknown air-like IDs are
  not guessed to be empty. True legacy numeric ID 0 with data 0 is
  already decoded by the native backend as ordinary vanilla air.
- Six existing native block-only operations accept
  `only_replace_air=True`: simple clone, repeated tiled clone,
  rotated/mirrored clone, XYZ integer-scale clone,
  rotate-then-scale clone and integer-reduction clone. For repeating
  overlapping stamps, the **last planned stamp wins**, but target
  eligibility is computed from the pre-write world, never from
  changes made by another stamp. Staged writes remain deduplicated
  by location so grouped Undo/Redo restores exact original targets.
- Original source BE positions are still **all** checked, including
  ignored source-air positions. A skipped occupied destination is not
  written and its full original BE NBT is left untouched; every actual
  native target to be written is still checked against BE conflicts,
  supported section/ID rules and full session bounds *before* any
  stage. A fully occupied destination makes a clean zero-change
  operation, without dirtying the session or creating an Undo entry.
- The Qt6 Tool Palette adds a persistent **Paste only into exact
  vanilla destination air** checkbox for all six clone functions,
  including the 3D preview/apply route. Changing the checkbox
  invalidates stale previews. Scaled, reduced and rotated+scaled
  dialogs expose the same option alongside source-air masking.
- `CloneGhost.only_replace_air` saves the mode in the read-only 3D
  plan; Apply re-runs native tests using the plan's saved flag.
  **Destination-masked previews are wireframe-only** because source
  voxel ghosts cannot prove native destination occupancy and could
  misleadingly depict writes into filled cells. The selection
  preview is never an authorization to overwrite blocked positions.
- Added `tests/test_clone_target_air.py` source regression cases
  specifying exact vanilla air identity rules, occupied original
  target/BEs remaining unchanged, source BE rejection, frozen
  destination occupancy during overlapping repeated stamps,
  independent source/destination masks, invalid target section
  preflight/atomicity, all six mode dispatch, preview flag
  validation, original Anvil modded BE/unknown tag preservation,
  save/reopen and Undo/Redo after Save.

**Not yet supported:** collision-aware per-voxel 3D native target
preview, migration of modded BE/Entity reference graphs, full
legacy original CloneTool rendering/tool parity, arbitrary modded
air definitions, full version-specific chunk semantics. New tests
remain source-only and have not been run as complete Qt/Mesa/world
CI or Windows packaging checks. Work remains only in the existing
Draft PR #1 / `python3-modern-editor`; `master` is untouched.

## 0.3.0a59 — Read-only original Java biome inspection

Added a current-Y native chunk biome inspector that reads the ORIGINAL chunk
NBT from the existing Java Anvil/MCRegion adapter without materializing
new chunks or calling any block/biome writer. The Qt6 File menu provides
**Inspect original biome slice (read-only)** for the viewport's chunk/Y.

- Java 1.18+ section `biomes.palette`/`data` is decoded as 64
  4×4×4 quart-biome cells using padded native long packing. Constant palettes
  with omitted data are accepted; missing/duplicated sections, invalid IDs,
  wrong array sizes, nonzero unused packed bits and malformed namespaces
  refuse inspection rather than fabricating values.
- Older Java `Biomes` ByteArray[256], IntArray[256] and
  IntArray[1024] formats are sampled at their original 2D column or 3D quart
  resolution. Numerical IDs are explicitly labeled `legacy-id:N`, not
  converted to guessed current biome names; modern modded names are retained
  verbatim, including their namespaces.
- The report counts 256 block columns at the chosen Y and labels the
  world's chunk DataVersion and exact source encoding. It uses the original
  disk chunk, never staged block replacements or a rewritten biome palette.
  Native region fingerprint checks remain in effect.
- `tests/test_biome_inspector.py` adds synthetic NBT tests for modern
  quart geometry, modded palettes, legacy arrays, corrupt bit packing,
  duplicate/missing sections, and unchanged original world/BE bytes and
  session edit/Undo state during a read.

**Still missing:** editing or importing biomes, handling version-specific
registry remapping and datapack biome validity, biome overlays, multi-chunk
painting and real Minecraft-version game-world verification. The new source
tests are committed as specifications; complete runtime, Qt, Windows and
game-version CI execution remains deferred. Work is confined to the existing
Draft PR #1 and original branch, without touching `master`.

## 0.3.0a60 — Native Java quart-biome painting (guarded, experimental)

This increment extends the a59 **read-only** original biome inspector with
deliberately bounded **Java 1.18+ native 4×4×4 biome editing**, retaining the
original no-Amulet region writer and full-region backups. Older 2D/1024-int
numerical biome formats remain read-only to avoid speculative registry ID
migration. This is not a cross-version biome converter.

- `biome_edit.py` reads original section `biomes.palette`/`data` from
  existing chunk NBT, rejects missing/duplicate sections, unknown/ambiguous
  schemas, malformed names, invalid indices/padding and DataVersion mismatches.
  For a changed 4×4×4 quart cell, it re-encodes only the affected biome
  palette and packed long array; all unrelated section tags, lighting,
  heightmaps, block states, BE inventories and original modded NBT survive
  byte-for-byte. A new exact name can be appended to the existing palette
  only while within the native 64-entry bound.
- Qt6 Edit menu and Tool Palette expose **Sample original biome at view
  center** and **Paint sampled biome to selection**. Sampling reads an
  actual original registry name from this same world/dimension (including
  modded IDs). Switching worlds/dimensions invalidates the sample. Painting
  expands each partially selected biome cube to the *whole* 4×4×4 original
  cell and makes that expansion explicit in its confirmation dialog.
- Each operation is limited to **128 quart cells**; all affected source
  sections, target chunks, exact native IDs and packing are preflighted
  before mutating any staged state. Atomic **in-memory batch** staging and
  one grouped **Undo/Redo**, including after Save, avoid partial quart edits.
  At Save the common original-region transaction independently revalidates
  from on-disk originals, backs up every affected original region and retains
  its ordinary rollback rules (not crash-atomic across separate files).
  Pending biome edits also prevent unsafe native backup restoration.
- `tests/test_biome_edit.py` specifies original/negative quart geometry,
  append-and-repack vs unchanged palette, strict validation, all-or-nothing
  cross-chunk failure, bound protection, Undo/Redo, save/reopen/backups,
  preservation of unrelated modded BE bytes and coexistence with normal
  block edits.

**Limitations:** Only original Java 1.18+ existing chunk biome sections;
no old numeric biome-ID writes, custom biome registry validation, automatic
datapack reconciliation, entity or BE semantic migrations, missing chunk
creation, smooth biome painting, visual biome overlays or tested cross-game
compatibility guarantee. The GUI only paints sampled names that actually
occur in this same world; the backend accepts exact explicitly specified
namespace strings but cannot prove they are registered for the destination.
The new regression cases were committed as source only; full Qt/Windows/
game-version testing remains deferred. This is still Draft PR #1 on
`python3-modern-editor`; `master` remains unchanged.

## 0.3.0a61 — Read-only 2D original/staged biome overlay

Implemented the missing **2D Java 1.18+ quart-biome visualization** for
the existing Python 3 / Qt6 slice view. This is *not* a new biome writer
or a 3D biome renderer.

- The View menu now provides a checkable **Show original/staged Java biomes
  (2D only)** overlay and **Biome overlay legend / diagnostics** dialog.
  At each view Y it depicts the original 4×4×4 biome quart cells, preserving
  exact modded biome registry names as read from native NBT, while showing
  staged unsaved names for changed quart cells. Each cell uses a stable
  original-name-derived display color; a gold outline distinguishes staged
  changes. The legend reports matching name/color codes and counts.
- An existing chunk's Section biome palette is decoded at most once per
  bounded viewport snapshot, not once for every onscreen block. Maximum
  visible chunk scan: **64 chunks**; larger scans are explicitly refused.
  The cached snapshot is invalidated by viewport movement/Y changes,
  world/dimension swaps, edit actions, Undo/Redo and Save. This is only a
  rendering cache and never writes/churns world NBT.
- Missing chunks, absent biome sections, native version mismatch and
  malformed/unknown original palette data are represented as conspicuous
  red crossed region rectangles; no false `plains` default or new chunk
  generation occurs. When the entire scan is refused, the 2D viewport
  displays an explicit error tint and message rather than silently drawing
  incomplete biome colors. Biome overlay is not available on numeric-ID
  legacy or MCRegion worlds.
- Added `tests/test_biome_overlay.py` source specifications for exact
  quart geometry, staged/original comparison, cross-chunk and missing
  data differentiation, bounds, original modded NBT preservation and
  post-save backed-up results. Added
  `tests/test_biome_overlay_qt.py` offscreen Qt controller source cases
  for cache reuse/invalidation, changed-state display and no edit history
  changes from enabling/displaying the overlay.

**Still missing:** visual 3D biome volume slices, zoomed-out biome map
overview, biome search/navigation, registry/data-pack validation and
original MCEdit2 feature parity. Original block/BE/entity/lighting
content is not modified by this visualization. Tests added as source only;
full Qt/Mesa/Windows and real Minecraft-version runtime runs are deferred.
Development remains confined to existing Draft PR #1 and
`python3-modern-editor` — no new PR, branch or `master` write.

## 0.3.0a62 — Bounded 3D original Java biome find and navigation (read-only)

Adds **exact namespaced biome ID search** in original Java 1.18+ Anvil
worlds. The existing 2D biome overlay, quart writer and original raw
NBT/Block Entity preservation remain unchanged.

- New `biome_search.py` scans only **existing** Java section biome
  palettes at native **4×4×4 quart resolution** across a user-selected
  *three-dimensional* selection. If nothing is selected, the UI restricts
  search to the current chunk at the current Y rather than scanning the
  world. Each original chunk and section is decoded once per scan.
  The absolute limits are **32 horizontal chunks**, **8192 quart cells**
  and **256 displayed hit coordinates** per query. A capped hit list
  retains the exact *total match count* for successfully decoded cells.
  Inputs and version are validated before any chunk read.
- Matching is **exact** against the original namespaced registry ID;
  modded IDs are supported verbatim with no guessed mappings, names
  autocomplete, wildcard expansion, DataFixer or registry interpretation.
  The dialog offers `include staged biome edits`: when checked,
  unsaved in-memory names replace original names for comparison while
  the search remains completely read-only.
- Missing original chunks, missing requested Y sections, mixed/invalid
  chunk schemas, malformed packed quart palettes and mismatched
  chunk coordinates/versions are reported per chunk/section. Decoded
  sections still contribute results; the final report explicitly marks
  incomplete searches instead of treating a failed read as a proven
  negative. The feature never generates missing terrain or sections.
- The Qt6 Edit menu and persistent Tool Palette expose
  **Find exact native biome in selection**. A results dialog displays
  match totals, partial-read diagnostics, a copyable report and at most
  256 navigable quart origins. Double-click/Go to result switches to the
  2D slice, pans to the hit, sets view Y and selects exactly its 4×4×4
  biome cell, without changing blocks, biome data, entities or undo
  history. Navigation refuses stale world/dimension results.
- Added native synthetic regression **source** `tests/test_biome_search.py`
  for quart X/Y/Z ordering, vertical slices, modded IDs, staged vs
  original search, Undo, cross-chunk completeness, missing source data,
  exact hit counting when truncated and invalid/oversized bounds.
  Added `tests/test_biome_search_qt.py` offscreen Qt source for
  navigation geometry, invalid locations and legacy refusal.

**Still not supported:** old numeric biome ID search, Bedrock worlds,
biome registry/data-pack validation, unbounded entire-world indexing,
3D volume overlay or general modded block-entity relationship repair.
Tests are committed as source, not reported as passing a full
Windows/Qt/Minecraft version matrix. Stay on Draft PR #1,
`python3-modern-editor`; `master` unchanged.

## 0.3.0a63 — Explicit original Java Block Entity structure transfer (experimental)

Closes a substantial GUI parity gap: previously the generic `.nbt` /
`.schem` / `.schematic` structure exporters correctly **refused** any
selection with Block Entity NBT. Instead of weakening their lossless
protection or silently stripping modded machines, this increment adds
a **separate, clearly marked MCEdit2 original-BE transfer workflow**
(`structure_be.py`). Vanilla/Sponge/classic block-only import/export
defaults and their BE refusal behavior remain unchanged.

- Qt6 File menu provides **Export selection with original BE NBT**
  and **Import original BE transfer**; both are explicit expert actions
  with irreversible modded machine/UUID ownership reference warnings.
  Export produces a gzip Java-structure-shaped `.nbt` with an
  `MCEdit2Transfer` marker and the exact source absolute origin.
  This app-specific transfer file is **NOT promised** to work as a
  generic Minecraft vanilla template and should not be used as one.
  Only the dedicated importer accepts its exact marker; generic
  structure NBT containing BE still refuses through its old path.
- Source block states use the existing bounded `structure_io` codec.
  Original BE Compounds are attached to each corresponding block under
  `nbt` with **raw opaque original bytes**, not deserialized and
  reconstructed field-by-field. Preserves unknown/modded inventories,
  energy, capability tags, owners, UUIDs and custom fields unchanged.
  Strict limits: **32,768 blocks**, **512 BE records**, **8 MiB total
  BE payload**, plus existing NBT/compressed file size caps.
  The writer verifies a full decode/encode roundtrip before atomically
  replacing the export file.
- Dedicated importer requires an **exact target DataVersion**, native
  named Anvil and existing target chunks/sections, rechecks all target
  positions and validates the original source coordinate-bound BE
  payload. Only the BE Compound's **three root TAG_Int x/y/z tags**
  are relocated, retaining every other byte. Any preexisting target BE
  aborts import. Entire target selection is preflighted before staging;
  successful transfers are a **single undoable transaction** backed by
  the existing original-Region save/backup/revalidation/rollback path.
- **Critical semantic limit:** Modded links, block/network references,
  owner/identity/UUID fields, inventories, and multiblock machine
  semantics are *not remapped or validated*. Copying unique modded
  machines may duplicate content or break links. Use disposable
  offline worlds only. There is no silent cross-version DataFixer.
- New `tests/test_structure_be.py` source regressions specify byte-exact
  BE transfer and opaque content preservation, marked-format refusal
  for generic import, gzip file roundtrip, existing target BE rejection,
  exact DataVersion validation, missing target section/chunk rejection,
  a mixed target-cross-chunk atomic refusal, original region backups,
  post-save Undo and save/reopen preservation of original BE coordinates.

**Remaining:** vanilla/Sponge/native external BE import compatibility,
general BE link/UUID rewriting, comprehensive real modded-machine tests,
Bedrock support, unrestricted entity structures and original GUI feature
parity. This is an explicitly opt-in experimental operation; runtime
Qt/Windows/Minecraft-version full testing is deferred. Keep Draft PR #1
on `python3-modern-editor`; no new PR and no `master` changes.

## 0.3.0a64 — BE-preserving native selection Move Tool, plus strict marked-file guard

Added a separate, **explicit BE-aware Move selection by XYZ offset**
alongside the older **block-only Move selection** (which still rejects
any Block Entity). Implemented `EditSession.move_box_with_nbt` and
Qt6 Edit-menu / Tool Palette expert entries. Source snapshots and
destination preflight are bounded by the existing editor's
32,768-block operation budget. This moves *blocks and their embedded
Block Entity payloads only*: standalone entities, scheduled ticks,
multiblock/network relationships and other dimension data are NOT
transferred.

- Source and destination *including overlapping regions* are
  snapshotted/validated **before** any staging. The move uses frozen
  originals, clears source-only cells to native air and stages
  destination snapshots with unchanged native BlockStates.
  Original BE opaque payloads are not normalized or converted; only
  strict root TAG_Int x/y/z are relocated to each new destination.
- Refuses invalid selection geometry/offset or out-of-bounds world
  points, missing original chunks/Sections, unsupported old numeric
  backends, unknown/malformed BE identities, or a target BE **outside
  the original source selection**. A BE already *inside* the
  overlapping source is allowed to move, preventing mistaken
  rejection of legitimate overlap cases. Ordinary occupied
  destination **blocks** outside the source are replaced as in the
  original MCEdit move tool, with explicit GUI warning.
- One complete native stage = one Undo/Redo item; exceptions trigger
  the existing reverse-staging rollback. Saving continues to require
  explicit confirmation and uses the existing full original-region
  transactional backups. Moving a modded machine retains, but does
  NOT repair, unknown links/UUIDs/inventories/capabilities or
  ownership. Must only be used on disposable OFFLINE copies.
- Also **fixed a63 file-type isolation**: ordinary `decode_structure`
  now explicitly refuses *every* marked BE-transfer template,
  **even if it contains zero BE records**. The expert BE-transfer
  decoder removes its marker internally before invoking the normal
  conservative block-format parser. Default vanilla/Sponge/classic
  block-only import behavior is otherwise unchanged.
- New `tests/test_move_be.py` synthetic disk-backed regression
  source covers two adjacent modded BE payloads, one-cell overlap,
  frozen-source move, copy of root coordinates only, independent
  block/BE state, staged vs disk, Save/backup/Undo/Save/reopen,
  external BE protection, absent target chunk/section and
  invalid/legacy geometry. New `tests/test_move_be_qt.py`
  offscreen Qt source covers explicit confirmation, selection
  navigation and no disk changes before Save.
  `tests/test_structure_be.py` also exercises the zero-BE marker
  import guard.

**Still pending:** real Java modpack compatibility testing, 3D BE
wireframe selection feedback, fully general modded BE identity/link
reparenting, native entity movement with selection, old numeric
schematic BE transfer, complete MCEdit2 GUI parity and comprehensive
Windows/Qt/Minecraft-version testing. Tests are source specifications,
not claimed to have passed a real-world test matrix. Existing Draft
PR #1, `python3-modern-editor`, `master` unchanged.

## 0.3.0a65 — BE-preserving original Java Clone Tool / repeated tiling (expert)

Ported another important original MCEdit2 editing flow beyond the
previous BE-aware Move Tool: **Clone selection WITH original BE NBT
(expert)**. This is a separate, explicitly confirmed native named-Java
Anvil operation; the existing block-only clone modes remain BE
protected, and generic clipboard/structure imports remain unchanged.

- `EditSession.clone_box_with_nbt(box, offset, repetitions=1,
  skip_air=False, only_replace_air=False)` captures a **single frozen**
  original source snapshot, including original/staged opaque modded
  Block Entity payloads, and stamps it at each **N × XYZ offset** for
  1–32 copies. Later repeated stamps win at duplicate destinations.
  All source/target coordinates, *existing native sections*, native
  BlockStates and BE payload identities are checked in advance; no
  chunk/section is created and no world data is converted.
- **Actual destination writes never overwrite an existing BE**,
  including source cells when a target overlaps its origin. A no-write
  result adds no Undo entry. A destination-air-only mask filters based
  on original target state **before any new clone staging**; source-air
  skipping omits exact vanilla air/cave_air/void_air but never drops
  a BE-bearing source cell. Ordinary occupied non-BE targets are
  overwritten unless masked. A failed later stamp rejects the entire
  operation before staging. The source remains unchanged.
- Block Entity NBT is preserved as opaque original bytes. Only
  source-validated **root TAG_Int x/y/z** values are relocated to each
  new cell. No modded UUID duplication handling, structure metadata
  remapping, inventories, ownership references, scheduled ticks,
  standalone entities, DataFixer conversions or multiblock/machine
  links are repaired. Use disposable OFFLINE worlds only.
- Limits: existing **32,768 planned cell** budget; 1–32 repeats;
  **512 target BE records and 8 MiB original payload data** across
  copies. Full target preflight occurs prior to any staging. A
  successful clone is **one Undo/Redo transaction**, saved with
  existing native-region transactional backups and confirmation.
- Qt6 Edit menu / Tool Palette adds **Clone selection WITH original
  BE NBT (expert)** with separate repeat count, source-air skip and
  destination-air-only options, plus a required high-risk
  confirmation. `tests/test_clone_be.py` adds disk-backed native
  source-level regression coverage for exact NBT preservation,
  original-source overlap, deterministic last-stamp resolution,
  source/target masks, absent chunks, BE destination collisions,
  no-op handling, save/backup, Undo/Redo and reload.
  `tests/test_clone_be_qt.py` adds offscreen Qt confirmation and
  Tool Palette test source.

**Not yet migrated:** modded machine-link/UUID repair, BE-safe
rotations/reflections of unknown mod-specific data, full native entity
and scheduled-tick structure transfers, complete MCEdit2 GUI parity,
Bedrock and comprehensive real-world Minecraft version testing.
The test sources are committed; runtime test execution is **not
confirmed**. Draft PR #1 stays on `python3-modern-editor`, with no
changes to `master`.

## 0.3.0a66 — Native 3D original/staged Block Entity location markers

The streaming OpenGL editor now supports a new **read-only,
opt-in Block Entity 3D wireframe overlay** built on its existing
selection/clone-box wireframe GL pipeline. Unlike entity/BE move or
clone tools, these markers NEVER modify, serialize, normalize or
rewrite chunk NBT. The original Block Entity coordinates and ID
come straight from the existing native Java Anvil/MCRegion
chunk reader; unknown modded NBT fields remain opaque.

- New `be_markers.py` performs bounded original and in-memory staged
  BE scans in a **5×5 chunk area centered on the camera**, vertical
  **Y ±32**, with **at most 256 drawn markers**. It reads one
  original NBT Compound per chunk, checks original chunk xPos/zPos
  identity, BE record ID and coordinate types, rejects duplicated or
  off-chunk BE coordinates and overlays `_be_edits` without altering
  the source bytes. The report retains full status counts even if
  drawing is capped.
- Cyan boxes denote original BEs, gold boxes denote staged
  created/modified BEs and red boxes denote staged deletions.
  A wireframe is slightly larger than a voxel, so an opaque cube
  does not fully obscure its location. Staged deletion markers
  represent original coordinates rather than restored BE records.
  Missing/malformed chunks are **explicitly listed as unreadable**,
  never interpreted as verified empty. All reported IDs are
  original strings rather than normalized Minecraft registry mappings.
- The Qt6 View menu now offers **Show original/staged Block Entity
  markers in 3D**, plus a **Refresh 3D Block Entity markers /
  diagnostics** dialog. The dialog includes exact result coordinates,
  copyable raw diagnostics and clickable/double-clickable entries
  that navigate to the corresponding original XYZ block using
  the existing safe selection handling; navigation is read-only.
- Marker scan results are cache-keyed to **world session,
  camera chunk X/Z and exact displayed Y**. Streaming's existing
  45-ms timer only checks whether that key changed, rather than
  parsing NBT per frame or invalidating entire terrain meshes.
  Explicit edit / Undo / Redo / Save routes refresh the snapshot
  after native staging; the regular brush-commit path also refreshes.
  The GL code reuses existing streaming selection-buffer vertices
  and avoids any new GPU texture/Section cache work.
- New `tests/test_be_markers.py` synthetic source checks original
  modded BE identity, staged add/modify/remove statuses, vertical
  cropping, invalid staged coordinate identity, missing-chunk
  diagnostics, 8-float wireframe geometry and strict query bounds.
  New `tests/test_be_markers_qt.py` offscreen Qt source verifies
  action toggles do not create GL contexts/write worlds, dispatches
  explicit refresh to an existing 3D view and requires an active
  world/view to show diagnostics.

**Current limits:** This is a nearby-area visualization, NOT a
complete whole-map BE search, a data-fixer, an accurate machine
model renderer or a native Block Entity/standalone entity edit.
It does not rewrite UUIDs, links, inventories, lighting or
heightmaps. Full real modded-world, Qt/OpenGL hardware/Windows
testing is NOT complete; new test files are provided as
**source only**. Keep Draft PR #1 on `python3-modern-editor`;
`master` untouched.

## 0.3.0a67 — Cross-chunk native Block Entity ID find / read-only navigation

Ported a broader original MCEdit2 world-inspection workflow beyond the
previous nearby-only 3D BE marker overlay. **Find original Block Entities
by ID** is available in the Qt6 Edit menu and persistent Tool Palette.
Search uses the **native original Java NBT** from the selected X/Y/Z
region (or the current 16×16 chunk at all supported heights if no
selection). The displayed form accepts **exact namespaced original
BE IDs** or `*` for all types; optionally includes in-memory staged
creation, modification and deletion records. No Minecraft block or BE
NBT is converted, repaired, saved, regenerated or overwritten.

- New `be_search.py`: search across **at most 64 chunks**, with
  **256 listed results**, while keeping an exact total of the valid
  decoded matches. Each chunk is parsed once, checks original NBT
  xPos/zPos, strict existing BE `id` and TAG_Int x/y/z, and rejects
  duplicate/out-of-chunk records or malformed staged coordinates.
  Hard limit of 4096 merged BE records/chunk. Entries are filtered
  by the exact **inclusive XYZ selection**, not quart-aligned
  blocks or an entire matching chunk.
- Unsaved staged additions, modifications and removals show statuses
  `created`, `modified`, `deleted` separately from unchanged
  `original`. Equal-byte staged records report `original`;
  pending deletion exposes the old original ID explicitly, so the
  user does not mistake the record for a currently present BE.
  Optional original-only search bypasses the staged overlay.
- A failed/absent chunk produces an explicit per-chunk diagnostic
  and **no partial result from that chunk**. The report states
  requested versus successfully decoded chunks, complete/partial
  coverage, exact match count, list truncation and original BE
  coordinates. Missing reads are **never** treated as proof that
  a matching BE is absent. The bounded search is read-only and
  does not create world chunks.
- Qt6 results list supports **Go to selected BE / double-click**
  to select the exact 1×1×1 XYZ cell and center the 2D viewport.
  Search retains the original-world identity and refuses to
  navigate a result after switching world/dimension. Reports are
  read-only and copyable. No OpenGL context is required to search.
- **Bug fix from a66:** the 3D native BE overlay now renders
  original/staged wireframe boxes **even when no 3D selection exists**.
  Previously the generic selection-line helper returned early
  and omitted BE markers during ordinary world browsing.
- New `tests/test_be_search.py` synthetic disk-backed regression
  source covers named modded IDs, wildcard/negative matches,
  original-only versus staged add/modify/delete, exact XYZ
  filtering, missing chunks, invalid staged identities, duplicate
  coordinates, bounded searches, truncated result lists and
  preservation of original disk bytes. New
  `tests/test_be_search_qt.py` adds offscreen Qt search-form,
  result navigation and unselected 3D overlay geometry source.

**Remaining work:** world-wide indexed BE search with progress and
cancellation, native standalone entity selection/filters, complete
MCEdit2 tool parity, modded BE semantic/UUID/network repair, full
PySide6/OpenGL on-device testing and game-world format matrix.
This is a read-only finder with **source-level test specifications,
not claimed executed tests**. Original Draft PR #1 and its
`python3-modern-editor` branch remain unchanged in scope,
and `master` was not modified.

## 0.3.0a68 — Indexed generated-world Block Entity finder with cancel/progress

Expanded the bounded a67 XYZ original BE search into a **full
generated-world** search of the *current native Java dimension*.
This is an explicitly requested, read-only operation; it does
not materialize ungenerated chunks, alter modded NBT, normalize
registries, repair UUIDs, or modify original region files.

- New `be_world_search.py` reuses `worldstream.RegionIndex` to
  enumerate **only generated chunk coordinates** from original
  .mca/.mcr 8-KiB region headers. It reuses the strict
  a67 `find_block_entities` validator one generated chunk at a
  time rather than introducing a second incompatible NBT parser.
  Limits are **200,000 generated chunks / 8,192 region files / 256
  listed matches** (still counting all valid decoded matches).
  Excessively broad worlds fail closed with an explicit error,
  rather than silently returning a partial index as complete.
- Supports an exact namespaced native BE ID or `*` wildcard,
  original-only or a **copied staged BE snapshot** captured at the
  beginning of the search, and reports `original`, `created`,
  `modified`, `deleted` statuses without rewriting BE data.
  The scope is one active native dimension (not all dimensions).
- Per-chunk results are atomic: an invalid chunk, off-chunk BE,
  duplicate BE position, mismatched original xPos/zPos, or malformed
  staged payload contributes **no partial hits**. The result
  independently reports chunk-index count, examined chunks,
  successful reads, invalid region-header slots/files, failed
  chunks, and staged positions missing from the generated chunk
  index. Any such gap or cancellation is marked **INCOMPLETE**.
  The full invalid-header count is retained with limited report
  details. Search never treats an unindexed region as verified empty.
- `be_search_thread.py` provides a **separate QThread** that creates
  and owns an independent read-only `NativeAnvilWorld`; it does not
  iterate the UI's mutable native region caches. The main Qt thread
  captures original BE staging data at launch. The worker emits
  progress after each 8 decoded/attempted chunks, checks its
  thread-safe cancellation Event **between chunks**, and emits a
  partial result on cancel.
- Qt6 Edit menu and persistent Tool Palette expose **Find Block
  Entities in ALL generated chunks**. A nonmodal progress dialog
  includes cancel; the final result view lists native IDs/statuses,
  has a copyable coverage/error report, and navigates to exact
  coordinates in the original selected world (world identity
  rechecked before navigation). The user can continue interacting
  with other Qt panels while the indexed scan runs.
- A running scan **blocks Save and native backup restore** so this
  editor cannot mix original chunk reads with an in-progress disk
  write. Closing the editor requests cancellation and refuses
  destruction of a running QThread; finish/cancel before exiting.
  Opening another world invalidates the search's navigation
  identity, so stale results are discarded.
- `tests/test_be_world_search.py` adds synthetic disk-backed
  far-apart multi-region coverage, cancellation, empty-world
  cancellation, staged snapshot isolation, invalid region header,
  orphaned BE stage, result truncation, oversized requests,
  and malformed chunk rejection. `tests/test_be_world_search_qt.py`
  adds offscreen QThread progress, pre-cancel, isolated reader and
  safe close source. All are **source tests, not claimed run**.

**Limits still outstanding:** full GUI/3D parity with original
MCEdit2, genuine Java/Bedrock version/mapping coverage, gameplay
modded BE semantic UUID/link repair, entity+tick transfers,
gigantic-world search pagination beyond 200k generated chunks,
and real OpenGL / OS test matrix. Remains in original Draft PR #1
on `python3-modern-editor`, with `master` untouched.

## 0.3.0a69 — Read-only generated-world Entity search (inline + split regions)

Added native Java **Find entities across ALL generated regions** in
the Qt6 Edit menu and Tool Palette. The implementation separately
indexes the old/original entity lists within generated terrain chunks
and the newer independent `entities/*.mca` files. Both storage
types retain their **distinct source identity**; no data is copied
from one to the other, no hidden chunk is generated and unknown
entity NBT bytes are not rewritten or normalized.

- New `entity_world_search.py` reuses the existing 8KiB-header
  `worldstream.RegionIndex` on both storage directory families,
  and `entity_index.read_entities` for original ID/Pos/opaque
  payloads. Original terrain xPos/zPos and available independent
  entity-region Position arrays are checked against indexed
  coordinates. Malformed, missing-ID or duplicated-invalid native
  records cannot produce a falsely complete negative search.
- Search exact original `namespace:entity` or `*`, across the
  currently opened dimension (not all dimensions); at most
  **200,000 indexed storage chunks**, **8,192 region files** and
  **256 listed results**. All successfully decoded matching
  records are counted, and partial coverage is explicit for
  unreadable original chunks, malformed index entries, staging
  orphan references, or cancellation.
- Optional **copy-on-start staged Entity snapshot** accounts for
  original, modified, created, deleted, transferred-out, and
  transferred-in records with their original storage source and
  list index. Transferred-out/in rows are *records about the
  staged movement*, not a claim that two live entities exist.
  No registry remapping, fake UUID creation or NBT rewrite
  occurs in the search. Results with missing, nonfinite or
  out-of-bounds Pos remain listed but cannot be navigated
  because a location must not be fabricated.
- New `entity_search_thread.py` creates an independent read-only
  `NativeAnvilWorld` on a QThread. The GUI supplies deep copies
  of the four staged Entity collection containers (payloads are
  immutable bytes). Search progress is delivered via Qt signals
  every eight storage chunks, with cooperative cancellation
  between chunks. A canceled result is always **INCOMPLETE**.
- Qt6 Edit menu / Tool Palette provide background search,
  nonmodal progress/cancel, exact-ID settings, copyable
  diagnostics, original source labels and navigation to
  the integer block containing a verified finite `Pos`.
  The original world/dimension must still match before
  navigation. Native Save, restore and window close now guard
  **both** Entity and Block Entity scan workers; closing a
  running worker requests cancellation first.
- Added `tests/test_entity_world_search.py` (disk-backed
  synthetic old/modern entity sources, exact-ID filtering,
  staged add/edit/delete/transfer, cancel, corrupt split
  files, orphan transfers, coordinate identity mismatch,
  missing native entity IDs and limits) and
  `tests/test_entity_world_search_qt.py` (offscreen
  native Qt worker, progress, precancel and close guard).
  These are **committed test sources, not runtime-confirmed
  passing tests**.

**Outstanding:** full GUI parity with old MCEdit2, native
entity painting/selection directly in 3D, version-by-version
Minecraft+mod validation, modded entity link and UUID semantics,
Bedrock support, and safe huge-world unlimited indexing. All
work remains in existing Draft PR #1 `python3-modern-editor`
and does not modify `master`.

## 0.3.0a70 — First full Python 3 CI run + private real-world archive audit

Ran a real GitHub Actions full top-level migrated Python 3 test suite
from the original Draft PR, instead of relying on documentation-only
regressions. Prior CI only ran a handpicked subset and normally skipped
draft PR updates. The `python3-modern.yml` workflow now also runs on
`python3-modern-editor` pushes and has a dedicated exhaustive
`full-modern-regression` job including native and offscreen Qt6
tests. Original Python 2 `tests/test_mceditlib` is deliberately
excluded from Python 3 compile checks; core tasks no longer import Qt
tests without installing PySide6. Old Python 2 library migration is
**not** declared complete.

The first true full run executed **873 tests** and exposed 7
regressions (5 assertions, 2 errors). Repaired native clone target
air-mask safety so it **never overwrites an existing Block Entity,
even if an invalid/modded world places the BE on an air BlockState**.
The other cases were incorrect/obsolete test expectations or
fixtures: 180° directional stair rotation, reduced clone's late
target write being an unmodified air no-op, immutable tuple-based
NBT `children`, byte-identical overlapping BE move write count,
expected expert transfer import error wording, and a Qt6 patched
dialog method signature. A repeat of all 873 tests plus three
core Python versions, Qt startup and Mesa GL smoke **succeeded**
before adding the new archive runner.

Two user-supplied **private real modded Java world archives** were
inspected locally without GitHub uploading them. Both archives have
root `level.dat` and original region files. Independent read-only
sector/decompression/NBT inspection found:
- **1.12.2 / DataVersion 1343**: 637 indexed terrain chunks,
  3,225 original Sections, 33 Block Entities and 304 inline
  Entities; zero malformed generated chunk NBT reads.
- **1.21.1 / DataVersion 3955**: 2,338 indexed terrain chunks
  (56,112 original Sections), 1,329 Block Entities, 155
  standalone Entity chunks with 405 records; zero malformed
  generated chunk NBT reads. It also contains **72 zero-byte
  .mca placeholders**, distinguished from actually populated
  or damaged generated chunks. Missing/invalid region headers
  remain visible in ordinary native index safety diagnostics.

Added `scripts/validate_real_world_archives.py`, an optional
**read-only real ZIP + native backend** smoke runner. It extracts
only level.dat/terrain/entity regions into a private temporary
directory; does not extract playerdata, modify originals, issue
Save or upload the archive. On a checkout with dependencies:
```
python scripts/validate_real_world_archives.py 1.12.2.zip 1.21.1.zip
```
It enumerates and parses EVERY generated native chunk, tests an
explicitly capped sample of actual decoded native BlockState
Sections, validates original coordinates and Entity NBT and
reports malformed *populated* region headers separately from
zero-byte placeholders. The test `tests/test_real_world_archives.py`
uses synthetic archives to run this script on CI. The provided
real-world ZIPs remain in the conversation sandbox, NOT in GitHub;
the **native editor backend smoke against those two user ZIPs
has not run on GitHub Actions**. The local ZIP inspection was an
independent read-only scanner, not a claim of a successful
in-game editor Save or complete world/version parity.

Version metadata in `pyproject.toml` was synchronized with
`modern/mcedit2_next/__init__.py` (now 0.3.0a70). Remains an
experimental Draft PR, with `master` unchanged.

## 0.3.0a71 — Actual private Java 1.12.2 / 1.21.1 native backend validation

**Executed the existing PR's native editor backend on both user-supplied
private worlds in this isolated session**. To overcome local GitHub
network access restrictions, a GitHub Actions job exported a short-lived
**source-only public artifact**, which was downloaded into the private
local test environment; the real worlds were NEVER uploaded to GitHub.
The exact `NativeAnvilWorld` backend on the public feature branch was
then run locally against two private archives:

- **1.12.2 / DataVersion 1343**: native `_chunk` parsed all 637 generated
  terrain chunks, decoded 128 sampled original block Sections, and
  the real native full-world BE/Entity search decoded all 637 chunks
  each, returning **33 original BE and 304 Entity records**, no failures.
- **1.21.1 / DataVersion 3955**: native `_chunk` parsed all 2,338 generated
  terrain chunks, decoded 128 sampled original block Sections;
  original BE search decoded 2,338 chunks and returned **1,329 BEs**;
  original Entity search decoded 2,493 storage chunks (2,338 terrain
  + 155 separate Entity) and returned **405 Entities**, no failed
  generated chunk reads.

A **real-world compatibility defect** emerged: 1.21.1 contains
**72 zero-length `.mca` region placeholders**. `RegionIndex`
previously classified these as damaged headers; both BE and Entity
full-world searches marked their coverage INCOMPLETE even though
every generated chunk was decoded successfully. The new index keeps
`empty` placeholder paths separate from `invalid` nonempty,
truncated or broken region headers. BE, Entity and ZIP audit reports
retain separate placeholder totals; they now correctly show
**complete generated-chunk coverage** for the 1.21.1 world.
These are not claims that empty files once contained data; true
nonzero-but-malformed headers must still cause incomplete results.
Synthetic regression sources cover all three paths.

**Native Save/Undo/Redo/reopen/backup tests on actual copies**:
Each real ZIP was extracted privately to a disposable temporary
directory; one already-generated block cell was changed with
`EditSession.paint` in a chunk containing real opaque BEs. After
Undo/Redo, Save, and a fresh native `NativeAnvilWorld` reopen:
- 1.12.2: **one actual opaque BE** in the edited chunk remained
  byte-for-byte unchanged.
- 1.21.1: **three actual opaque BEs** in the edited chunk remained
  byte-for-byte unchanged.
- The native region backup exactly matched the original pre-edit
  region bytes. Restoring that backup in the isolated test copy
  and opening the world again recovered the original block and BEs.
  The original ZIPs were never edited or uploaded.

A reusable `scripts/verify_native_roundtrip.py` now runs a guarded
one-cell native Save/Undo/Redo/BE/backup smoke against private
original Java ZIPs. It extracts only level.dat and overworld
terrain regions into a temporary directory; never touches
playerdata or the original archive:
```
python scripts/validate_real_world_archives.py 1.12.2.zip 1.21.1.zip
python scripts/verify_native_roundtrip.py 1.12.2.zip 1.21.1.zip
```
Added synthetic `tests/test_native_real_roundtrip.py` to CI,
along with zero-byte index and whole-world BE/Entity regressions.

**Limits:** These are read-only exhaustive generated-chunk searches,
128 sampled Section decodes per world and one reversible edited
block per version. Full 3D graphics on the actual saves,
every Section palette combination, modded machine graph semantics,
multi-BE move/clone, in-game verification and multi-dimension saves
remain unverified. Existing Draft PR #1 and feature branch are
retained; `master` unchanged.

## 0.3.0a72 — Actual multi-region native write/Undo/Redo and BE preservation regression

Extended a71's one-cell smoke into a bounded **real multi-region
editing test** using the original user-provided private Java world
ZIPs. Testing is performed **only on disposable extracted copies**;
the original ZIPs are SHA-256 checked before/after and not uploaded
or committed to GitHub.

On local real-world runs (Java 1.12.2 and Java 1.21.1):
- **1.12.2 / DataVersion 1343:** 9 affected generated chunks across
  2 original .mca regions, 9 edited batches, 26 affected block
  coordinates, 21 original BE NBT compounds individually compared
  byte-for-byte and preserved after Save/reopen.
- **1.21.1 / DataVersion 3955:** 9 affected generated chunks across
  4 original .mca regions, 9 edited batches, 26 affected block
  coordinates, **303 original BE NBT compounds** individually
  compared byte-for-byte and preserved after Save/reopen.
- For both worlds, the test selects existing BE-rich chunks
  across distinct regions, uses the normal EditSession Fill and
  Line API (including a **true adjacent chunk boundary**),
  replays all Undo/Redo operations, confirms no disk writes
  before Save, verifies the native backup matches every
  original affected region file, and checks the edited cells
  after a fresh native reopen. All non-Section original chunk
  NBT is compared byte-for-byte in every modified chunk.
  Original raw NBT for every **unedited generated chunk** in
  affected .mca region files is checked as unchanged. The
  restored backup bytes and original block states are checked
  after reopening a fresh backend.
- Added repeatable `scripts/verify_native_batch_roundtrip.py`
  with strict edit limits (at most 12 selected original BE
  chunks, eight changed region files, 96 unique cells);
  aborts rather than generating missing world chunks. New
  `tests/test_native_batch_roundtrip.py` provides synthetic
  GitHub Actions regression coverage with deliberately
  protected BE positions and cross-region generated chunks.

**Known limits:** This is still bounded Fill/Line of safe writable
original states, not arbitrary block-entity relocation, DataFixer,
full world rollback after every crash, giant multi-chunk editing,
or Minecraft game runtime/mod behavior. It exercises old numeric
and modern named BlockState formats on these two real private
worlds but does not establish all-version/machine semantics.
The 3D editor and entity transaction matrix remain incomplete.

Development remains in original Draft PR #1 branch
`python3-modern-editor`; `master` untouched.

## 0.3.0a73 — Real modded BE move/clone cross-region native roundtrip

**Executed actual user-supplied private Java world ZIP testing
locally**, using the matching native backend exported from successful
feature-branch GitHub Actions as a source-only artifact. Real ZIPs
and player data were never committed or sent to GitHub. All edits
occurred exclusively in disposable temporary world copies.

- Java **1.21.1 DataVersion 3955**: a dynamically selected
  original modded `lootr:lootr_barrel` Block Entity (202 bytes)
  was **cloned** and **moved** from an existing chunk on one
  side of a .mca region boundary to an already generated
  destination chunk in the adjacent region. Each operation
  preserved the exact original opaque NBT bytes except the
  three top-level TAG_Int x/y/z coordinates intentionally
  relocated. Undo, Redo, Save, fresh native reopen and
  byte-for-byte original region backup restoration succeeded.
- A second original high-payload BE, a `minecraft:mob_spawner`
  compound of **5,146 bytes**, was independently moved/cloned
  across a generated chunk boundary with the same checks.
  The whole NBT compound, including nested original data
  outside root coordinates, remained byte-identical to the
  coordinate-only expected relocation after disk Save.
- Four successful real-world transactions total: clone/move
  of each source; clone changes exactly one target block
  cell and only writes/backs up the target region; move
  changes two cells and writes the affected source/target
  regions. Tests check original staging is not written to
  disk until Save; source remains identical for clone and
  is cleared for move; Undo restores both original cells,
  Redo restores staged exact NBT, and backup restores
  original BE/source/target bytes on fresh reopen.
- In every original-world transaction, an attempt to clone
  or move onto an **already occupied original BE** was
  rejected without any staging/Undo entry. Attempts to
  move/clone outside native Java coordinates also failed
  without writes. All unrelated original BE compounds
  in the two touched chunks and all unedited generated
  chunk raw NBT in changed .mca regions were checked
  byte-for-byte after reopen.
- Java **1.12.2 DataVersion 1343**: numeric ID/metadata
  format BE relocation is not yet safely implemented by
  this editor. Both native BE clone and move were
  explicitly refused with no stage data, Undo changes or
  source ZIP writes. **This is a verified fail-closed
  guard, not implemented 1.12.2 BE migration support.**
- Added reusable **`scripts/verify_native_be_transfer.py`**
  selecting original candidate BE records from existing
  generated chunks, with a preference for a modded region
  crossing and the largest opaque BE payload.
  Hard original ZIP-path/extraction protections are reused
  from the a71 real-world runner, and output reports
  per-operation completion and safety constraints.
- Added synthetic regression **`tests/test_native_be_transfer_world.py`**
  with two generated chunks in separate .mca regions,
  BE preservation, Undo/Redo/Save/reopen/backup and
  numeric-format fail-closed checks. The a73 feature-branch
  CI run executed **887 migrated tests (95 modules), all
  passed**; core Python 3.12/3.13/3.14, Qt6 GUI and Mesa
  OpenGL jobs also passed. Initial private runner assumption
  that clone must back up the *unmodified* source region
  was corrected: clone backs up only the written target
  region; the source region must instead remain untouched.

**Remaining limitations:** No real modded-machine network/link
repair or UUID semantic migration; a BE may physically transfer
losslessly but be logically invalid in-game. No general legacy
1.12.2 numeric BE move/clone, full 3D interaction on these saves,
all-dimension edits, game-client load tests or exhaustive mod
compatibility matrix. PR #1 stays Draft on `python3-modern-editor`;
`master` was not modified.

## 0.3.0a74 — Native 1.12 numeric-ID Block Entity Move/Clone (no DataFixer)

**Added guarded original Java 1.2–1.12 Anvil (.mca) Block Entity
copy/move**, where the world still uses native Blocks/Data/Add arrays,
rather than paletted named BlockStates. Each source block retains its
exact original **numeric ID 0..4095 + metadata 0..15**, including
unknown/modded ID values and high ID bits carried by Add. Its original
opaque BE NBT is copied byte-for-byte except the strict root TAG_Int
x/y/z values. No mapping to newer Java block names, DataFixer,
modded capability reconstruction or in-game link/UUID repair happens.

- `EditSession.clone_box_with_nbt` and `move_box_with_nbt`
  now accept **explicitly capability-marked** original native numeric
  Anvil worlds. MCRegion (.mcr), synthetic/fake legacy backends
  without full native support and absent generated chunks/sections
  remain explicitly forbidden. Qt6 Clone/Move menu actions expose
  this capability after checking the native world adapter.
- Native `get_cell`/`set_cell` preserve the entire original
  numeric BlockState as an opaque id/meta pair and all original
  BE bytes. New BE data cannot be attached to ID 0; BE deletion
  requires replacing the original cell with numeric air (0,0);
  creation requires an actual change to the original numeric
  state. The reader rejects malformed source BE records, including
  real-world original BEs attached to numeric air, rather than
  inventing a plausible block ID.
- The legacy `patch_blocks` writer keeps its **default block-only
  BE overwrite prohibition**. Only a strictly controlled
  `allowed_be_positions` set of explicitly deleted BE source
  coordinates permits its original Blocks/Data/Add arrays to be
  updated. The native Save transaction requires an aligned
  original numeric block change for each created/deleted legacy
  BE, verifies the exact NBT id/coordinates and rejects an
  unpaired/orphaned payload before writing any region file.
  All other untouched BE records and non-block tags retain
  original bytes. Undo/Redo remain atomic with preflight
  guarding existing BE targets, source anomaly, missing
  regions/chunks/sections and coordinate bounds.
- The original private **1.12.2 DataVersion 1343** Java world
  now passes **four** real native Move/Clone roundtrips:
  two original `ebwizardry:bookshelf` BEs with opaque
  **792-byte and 771-byte** NBT payloads, each cloned and
  moved across existing generated chunk boundaries. After
  Undo/Redo and native Save/reopen, exact ID+metadata and
  relocated BE NBT match expected originals, and
  **377 untouched generated chunks** in affected regions
  retain byte-identical raw NBT for each operation.
  All nearby unrelated BEs remain unchanged, BE-target
  collisions are refused without staging, and original
  region backups restore the original bytes. Source ZIP
  files remain private and unmodified.
- The **1.21.1 DataVersion 3955** private world was retested
  using the same updated original backend: existing Lootr
  barrel cross-region and 5,146-byte original spawner
  copy/move cases still pass, as do refusal and backup checks.
- Upgraded `scripts/verify_native_be_transfer.py` to
  exercise both original numeric and modern Java region
  backends against private ZIP copies, instead of the
  prior numeric-only rejection. Added comprehensive
  `tests/test_legacy_be_transfer.py` synthetic cross-region
  ID=511, metadata=13, original high-bit Add regression,
  collision rejection, numeric-air anomaly refusal and
  block-only BE protection.

**Remaining scope:** This establishes **format-level byte-safe
1.12 Anvil BE transfer in guarded selected cases**, not automatic
reconstruction of modded multiblock logic, tick scheduling,
linked UUIDs, unknown machine adjacency semantics, older
MCRegion, game-client success or exhaustive versions/mods.
Never assume numeric ID 0 BEs are safely movable; those
remain refused. Draft PR #1 on `python3-modern-editor` retained,
and `master` remains untouched.
