# MCEdit2 Next — Python 3 migration (experimental)

This branch starts a new Python 3 / Qt6 editor **alongside** the unmodified
legacy Python 2.7 / PySide 1 MCEdit2 code. It is **not yet a complete port**.

## Included

- Python 3.12+ project configuration and `python -m mcedit2_next` entry point.
- Qt6 desktop GUI with **2D horizontal slices and an interactive OpenGL 3.3 3D voxel view**. Switch modes with the View dropdown; the 2D viewer remains a fallback.
- Paint, select (including vertical bounds), fill, replace, copy/paste,
  undo/redo, continuous-stroke undo and manual save.
- Version-neutral block operations: negative Y positions, namespaced block
  states and high-cardinality (up to 4096 entries) section palettes.
- Per-operation block count limit (default 32768) and best-effort rollback.
- Independent Python 3 NBT/Anvil backend for Java 1.13+ world edits, without Amulet.

## Install and run (development only)

```sh
python -m venv .venv
# Activate the virtual environment.
python -m pip install -e .
python -m mcedit2_next
```

The active Qt editor uses a native Anvil/NBT backend. The old optional Amulet bridge in worlds.py remains only as an experimental reference. Use only disposable world copies.

- New opt-in plugin manager: metadata discovery, load/unload/reload, legacy
  SimpleCommandPlugin adapter and Qt6 options forms; working brush modes.
- Read-only Block Entity NBT tree viewer (Edit > Inspect BE) and immutable
  raw NBT payload access for plugins, without exposing BE writes.
  Other legacy extension categories have registries but not yet GUI bindings.
  See [plugin migration and API](doc/modern_plugins.md).

## Important limitations

- The new 3D view draws exposed cube faces in simple colors and provides orbit, zoom, pan, 3D selection and ray-picked editing. **It is not a port of the full MCEdit2 renderer**: basic full-cube texture loading from explicitly added client JAR/resource ZIP/mod JAR, but no accurate stairs/fences/complex models, transparency sorting, animated materials, entities or sophisticated chunk meshing yet.
- Read-only BE NBT inspection and experimental BE-aware copy/paste are available. General entity editing, robust BE relationships, world revision history, and accurate modded block models are not implemented.
- The native backend has synthetic format tests but needs real Java-world fixtures and game-side verification before production writes.
- The native backend creates byte-identical original region backups in .mcedit2-backups before atomic region replacement. It is not a fully atomic multi-region transaction.
- Full Java + Bedrock + snapshots + modded compatibility is **not claimed**.
  Original Anvil is incompatible with Minecraft's newer palettes, heights,
  biomes and dimension system; do not route modern saves through it.

## Basic full-cube texture archives

Open **File > Add resource pack / client JAR…** and select the relevant
Minecraft client JAR or resource-pack ZIP. Additional mod JARs and resource
packs can be layered (later files override earlier assets). The 3D viewer
reads blockstates JSON and model texture references for simple full cubes.
Complex, rotated, multipart, missing or unsupported models use the same
color-cube fallback; texture load never rewrites any archive or world data.

## Headless tests

```sh
PYTHONPATH=modern python -m unittest discover -s tests -p 'test_modern_*.py' -v
python -m compileall -q modern
```

Next: real-world format verification, material and block model rendering, advanced 3D tools, blockstate/biome/entity preservation, legacy plugin migration and a versioned world fixture + CI matrix.

## Windows portable preview (experimental)

GitHub Actions workflow `Windows Qt6 preview build` builds a PyInstaller
one-directory executable for Windows x64 and uploads a temporary ZIP artifact.
Open the workflow run's **Artifacts** section to obtain the preview ZIP,
extract it, and run `MCEdit2-Next.exe` in the resulting directory.
It is **not** an official production release. Keep Minecraft closed and
edit only disposable copies of Java worlds. Automated compile/import tests
do not replace real-game save/reopen validation.

## Batched CI strategy

Development pushes to `python3-modern-editor` no longer launch the Python
matrix or expensive Windows preview builds. After a group of commits is ready,
open **Actions > MCEdit2 Next validation > Run workflow**, choose the feature
branch, and run once. Run **Windows Qt6 preview build** manually only when
you need a new ZIP, or push a `next-v*` preview tag.

Full Linux CI also runs when a non-draft PR is opened/marked ready, and on
pushes to `master`; draft PR updates are not automatically validated.
Further pushes to a ready PR do NOT automatically validate: manually rerun
before merging. Concurrency cancels older runs for the same ref.

## Java structure templates

The File menu supports block-only vanilla Java `.nbt` structure templates.
Exports check the selection for BE and refuse unsafe copies; imports reject
BE/entity payloads rather than silently discarding data. Import placement
is one undoable edit transaction. This is not Sponge `.schem`; importing
states from a different DataVersion requires explicit confirmation and
does not perform Minecraft DataFixer conversion.

## Sponge v2 schematics

The same File menu imports/exports block-only `.schem` files in Sponge
schematic v2 format (namespaced block-state palette + VarInt BlockData).
Uses WorldEdit coordinate ordering. Entities, block entities and biome
payloads are rejected until they can be moved without loss. Sponge v3 and
legacy numeric-ID `.schematic` files are not yet supported.

## Trigger batched checks while this PR is draft

GitHub may hide `workflow_dispatch` when its YAML has not yet reached the
repository default branch. For the current draft PR, trigger the complete
Linux validation by applying the **`run-next-ci`** label. Trigger a portable
Windows build by applying **`build-next-windows`**. To rerun either,
remove then add its label again after your next batch of changes. Other PR
labels do not start test jobs. Once the workflows are on `master`,
the **Run workflow** button is also available.

## Progressive chunk-streamed 3D view (post-0.2 alpha)

The former prototype iterated every block in a tiny cube and regenerated
the full mesh/texture atlas synchronously on repaint. The new experimental
3D mode uses the entire generated-chunk **region header index**, loads
existing 16^3 Sections on two background decode workers, builds meshes
incrementally, caches persistent per-Section GPU buffers, and uses a
grow-only stable UV texture atlas. Camera orbit and zoom update matrices,
not block/NBT decoding. Neighbor Section data removes hidden boundary
faces. Missing mod textures are magenta/black instead of invisible blocks.

The **3D chunk radius** now measures *chunks* (16 blocks each), rather than
an 8-block sample cube. Farther radius is progressive; it may take time to
load. **RAM cache** is a limit on decoded Section count, not an exact byte
cap. The whole map's generated chunk coordinates are indexed, even when
distant block data is not resident in RAM. Missing/unwritten chunk sectors
are not generated by the renderer.

**Edit > Go to coordinates** (Ctrl+G) repositions camera/selection layer.
**Edit > Preload visible Sections** requests the current view in the
background. **Edit > Preload whole generated map** scans all existing
chunks and tries to load all their Sections, but only if their number
fits the selected decoded Section cache. Otherwise it explicitly refuses
bulk residency while leaving normal streaming/whole-world indexing
available. Large/infinite/anomalous maps are not forcibly decoded into
unbounded RAM.

UI localization currently includes English and Simplified Chinese for the
main menus and editing controls; extra string packs can be installed at
`~/.mcedit2-next/locales/<locale>.json`. Some dialogs/plugin text remain
English and are scheduled for translation.

### Known limitations

- Not yet the Minecraft render engine: greedy meshing is implemented, but
  multi-distance LOD, occlusion culling, translucent passes, complex
  noncubic models, and full shader/light updates are not. Section-level draw calls
  can still become expensive at very high radii.
- Background streaming does not certify map save integrity. Preserve a
  full offline backup. Real-world regression fixtures are still required.
- Pre-1.13 Java Anvil support is guarded numeric-ID editing only; unlisted
  modded IDs render as opaque legacy placeholders and cannot be silently
  remapped to modern blocks. MCRegion .mcr and Java Classic files need
  independent adapters.

## 3D greedy meshing and asynchronous GPU preparation

The progressive 3D renderer now merges coplanar block faces with the same
blockstate/atlas tile into rectangles. A solid 16³ Section requires just
six quads (36 vertices), rather than 1,536 quads (9,216 vertices) for
all exposed single-block faces. The dedicated shader repeats texture UVs
across each merged rectangle, avoiding stretched textures. Neighbor Section
borders are still culled; missing Sections render as air rather than hiding
neighbor faces. This is a *cuboid-face mesher*, not a full Minecraft
multipart geometry renderer.

Greedy mesh generation now runs in a bounded Python background worker
queue, while atlas updates and OpenGL vertex buffer uploads remain in
the Qt GUI thread. Generation numbers discard obsolete mesh results
after edits/world switches. Cached decoded Sections can rebuild evicted
GPU meshes on returning to a location, without another disk read.

Further work is needed for aggressive multi-Section batch drawing,
translucent face sorting, full LOD hierarchy, complex block geometry,
and actual large modded-world profiling. The system still rejects
out-of-budget memory residency instead of allocating unlimited RAM.

### Migrated shape-editing commands

The Edit menu now offers hollow/shell selection, solid/hollow spheres,
and 3D lines between selection corners, alongside existing fill, replace,
paste and undo/redo operations. They respect the per-operation edit cap
and native BE protection; a failed operation is rolled back.
Plugins can stage sphere edits through the plugin context.

## Experimental BE-aware clipboard

Ordinary Copy/Paste remains block-only and still refuses BE positions.
The explicit **Copy with BE NBT** / **Paste with BE NBT** tools stage a
snapshot of the original named block state and opaque BE compound payload
together. Only three root TAG_Int coordinates (x/y/z) are relocated. All
unknown modded NBT fields remain byte-identical; no change is made to
nested UUIDs, linked machines, inventories, networks or ownership metadata.
Pasting over an existing BE is refused. BE operations use undo/redo and
the native Anvil backup/save pipeline, but this is **experimental** and
not validated against arbitrary modded machines. Pre-1.13 BE transfer is
explicitly disabled, even though limited old numeric block viewing works.
Use offline disposable copies only.

## Legacy MCEdit Alpha .schematic migration

Classic .schematic is now a distinct block-only import/export format alongside
vanilla .nbt and Sponge v2 .schem. It reads Blocks, Data, and optional
AddBlocks in the historical Y-Z-X order. Only well-defined numeric IDs and
metadata variants are translated; unknown modded IDs, direction-dependent
metadata, entities, BE records, nonzero WorldEdit offsets and unexpected
custom fields explicitly refuse conversion. This is not a DataFixer and does
not support the complete original legacy schematic adapter.

See MIGRATION_STATUS.md for the full audit of migrated/partial/missing pieces.

## Native version preservation (not world conversion)

The editor changes only known fields in the input format. Legacy Anvil
retains `Blocks`, `Data`, optional `Add` and all unknown NBT tags, including
modded numeric IDs 0–4095 and native metadata 0–15. Enter native values as
`mcedit2:legacy_id_511[data=9]`. A high ID creates a legacy `Add` byte array
inside that Section if absent, not modern blockstates. The app does not write
`level.dat`, refuses to save when it changed externally, and verifies chunk
`DataVersion` remains unchanged on native legacy writes. This is not a
DataFixer and does not translate a 1.12 world into a newer world version.

## Native entity inspection without conversion

Read-only entity listing now handles historical `Level/Entities` inside
block chunks and modern standalone `entities/r.X.Z.mca` regions. The UI can
inspect original entity NBT in the current chunk without changing any tag.
This feature is intentionally not a serializer, UUID converter, or entity
move operation. Entity editing/writing is still unavailable.

## Early MCRegion native support and block sampling

Old Java `.mcr` region files now use an independent 16×128×16 block codec:
original X-Z-Y byte arrays, 8-bit IDs and 4-bit metadata are edited without
upgrading to Anvil. Unknown NBT, tile entities, entities, light and heightmap
remain unchanged. The editor rejects ambiguous old worlds containing both
`.mcr` and `.mca`, or a modern DataVersion with only old `.mcr` regions.
The visible Y editing range is 0–127 for MCRegion. Legacy worlds display
native numeric ID and metadata controls, and right-clicking a block in the
2D slice samples its original ID/metadata. `Ctrl+Shift+B` samples the view
center in either mode. This does not change world versions.

Known limitation: basic block placement does not recalculate old lighting or
heightmaps. Actual game roundtrip on old worlds is still needed.

## 0.3.0a2 — Java MCRegion and legacy editing UI

The editor can read and patch **original** Beta 1.3–1.1 `.mcr` regions,
preserving the old 16×128×16 X-Z-Y chunk layout (only block ID and four-bit
metadata arrays are edited). Existing light arrays, heightmap, tile entity,
entity and unknown custom NBT remain byte-identical. Saving keeps `.mcr`
extension and the exact original `level.dat`; no Anvil version conversion
or generated chunks. Mixed `.mcr`/`.mca` old worlds are rejected as
ambiguous rather than silently selecting one.

The UI provides native numeric ID and metadata input (ID 0–255 for MCRegion,
ID 0–4095 for old Anvil), MCRegion height 0–127, right-click 2D block
sampling and Ctrl+Shift+B sampling at view center. For maps needing accurate
lighting/heightmap recalculation, full compatibility is *not yet* verified.

## 0.3.0a3 — Native selection offset tools

Clone and Move Selection operate over an explicit X/Y/Z offset, snapshotting
all source blocks *before* writing. This fixes the classic overlap hazard
where a moved source could overwrite subsequent reads. Moving clears
source-only positions to native air and writes target positions in a single
undoable transaction; cloning leaves the original unchanged. Bounded 6-face
flood fill uses the selected 3D volume as a safety boundary.

All operations retain the world's original state representation
(namespaced modern properties or legacy numeric ID+metadata), and perform
read-only target validation before staging changes. They deliberately
reject protected Block Entity positions and do not relocate entities or
update modded multiblock links. This is not an entity/BE move tool and
does not convert Minecraft world versions.

Lighting and heightmaps are currently preserved, not recalculated; edits
that modify blocks may therefore require version-native relighting
before use in Minecraft. Real-world validation is still required.

## Original-format horizontal transforms

Rotation/mirror of selected blocks now understands a conservative verified
set of original-version vanilla direction fields. Clockwise/counterclockwise
90-degree operations transpose the X and Z extents within a bounding box
anchored at the original minimum coordinates. X and Z mirrors retain extents.

The implementation changes supported modern blockstate fields and a small
stable pre-1.13 numeric metadata set. It does **not** convert legacy IDs to
modern names, rewrite modded fields or rotate Block Entities/entities. Unknown
state or metadata semantics, protected BE coordinates, missing Sections and
over-budget edits fail before any edits are staged. Successful transforms
create a single undoable transaction. Importantly, unknown modded blocks are
not considered orientation-independent by default.

Remaining limitations: validated transforms do not cover every vanilla
multipart block, modded directional state, paired BE, sign/block-entity content,
re-lighting or heightmap recalculation. Back up an offline copy first.

## Native entity positions without version conversion

An experimental option in the entity inspector can edit X/Y/Z positions
of some simple standalone vanilla entities in their **existing chunk only**.
The original NBT Pos TAG_List of exactly three TAG_Double members is patched
byte-for-byte in place, without modifying an entity's ID, UUID, inventories
or unknown fields. Both old inline `Level/Entities` and modern split
`entities/r.X.Z.mca` write to their original paths through existing
backups. The edit participates in normal undo/redo, including after saving.

Unknown mod entities, mounted/attached/owned entities, non-double Pos data,
cross-chunk moves and field mutations other than Pos are rejected before
mutation. Lighting, world entity loading and modded interactions still
require real-game validation.

## 0.3.0a6 — Original NBT scalar editing for existing Block Entities

The existing NBT tree now has an opt-in **Edit selected scalar** action for
BEs. It uses a strict value parser and original-tag-kind-preserving byte-span
patcher, not full compound serialization. Unknown fields, byte arrays,
inventory data, capabilities, nested lists, UUID and coordinates remain
unchanged unless the user explicitly targets a permitted scalar. Identities,
BE x/y/z, ownership references, attachments and other protected names cannot
be changed through this tool.

The native writer permits in-place scalar edits on original-version
`.mca` (both paletted and numeric) and `.mcr` without converting the map.
The edit participates in undo/redo and backup, including after saving.
Untouched chunk data remains byte-identical.

**Do not change arbitrary mod-specific numbers unless you understand their
semantics.** Invalid tag kinds/ranges and unsupported layouts are rejected,
but legal numbers may still be invalid to particular mods; use an offline
world copy and validate in the original Minecraft version.

## 0.3.0a7 — Packed NBT array member editing

The Block Entity viewer now supports replacing one existing member in a
Byte_Array, Int_Array or Long_Array at a specified zero-based index.
The original 1/4/8-byte member is patched without repacking the array,
changing its length, or rewriting unknown NBT. A bounded index prompt
avoids expanding large arrays into thousands of Qt items.
Undo/redo and original-version region backups are reused. Values may be
syntactically valid yet semantically invalid to specific mods.

## 0.3.0a8 — Surgical NBT structural editing

Expert-only original-format NBT editing now includes explicit addition and
deletion of scalar fields in an existing Compound, insertion and deletion of
members of scalar-typed Lists, and insertion/deletion of individual Byte, Int
and Long Array members. These are **structural** changes performed by byte
splicing only the affected compound end marker or collection member and
count. Untouched tags and unknown mod data are never reconstructed.

Protected BE identity, coordinates, UUID, owner and relationship field names
remain inaccessible. List element kinds are retained. Empty TAG_List with
unknown element type is not automatically guessed. No arbitrary creation
of Compound/List tags, BE relocation or format conversion is implemented.

The UI requires explicit selection and confirmation for destructive actions;
all edits join the normal undo/redo plus native region backup workflow for
modern and old Java Anvil and MCRegion. This does not guarantee that a mod
will accept changed collection sizes: only modify offline test copies.

## 0.3.0a9 — Restoring native editor region backups

An explicit File-menu recovery action lists snapshots created by MCEdit2
Next region saves. A selected snapshot can be restored only with no staged
edits and unchanged world metadata/session. The editor backs up the current
native region files into a NEW snapshot before replacement, preserving an
inverse recovery point. `.mca` and `.mcr` files retain their names and
binary format; `level.dat` is untouched.

Restore operations verify snapshot origin, file names and target paths; they
refuse symlinks, missing regions, concurrent world changes and unsupported
file types. The GUI clears old undo/redo state and refreshes caches.

Warning: multiple region files cannot currently be restored crash-atomically
if the OS or computer stops during the write. Test only offline copies.

## 0.3.0a10 — Python 3 generator/tool plugin callbacks

Qt6 Plugins menu now offers registered native Python 3 tool callbacks and
bounded world generators. Modern plugins may register `host.register_extension`
with category `tool` or `generate` and a callable `(ctx, options)`. Their
writes are staged in a single `EditTransaction` and can be undone. Generator
callbacks require a selected region and inherit operation budget limits.

The bundled selection-tools example exposes a checkerboard generator that
can place blocks directly in an original-version world without translating
storage formats. Registered *classes* from old PySide1-based MCEdit2 tools
and generators remain intentionally uncallable; they require an explicit
Qt6/editor-world API migration rather than blindly invoking legacy classes.

Only trust third-party Python plugins you have inspected; plugin code has
normal Python process privileges and is not sandboxed.

## 0.3.0a11 — Original plugin feature migration: inspectors and shapes

Python 3 plugins may now register an `inspector` callable receiving a frozen
`BlockInspection(position, state, block_entity)` snapshot. The Qt6 Plugins
menu renders a bounded read-only text result or small scalar field map;
the BE payload is opaque original NBT and is never normalized.

Native `brush_shape` callbacks receive `(ctx, point, block)` and stage
spatial stamps within one drag transaction, preserving undo and existing
region/BE preflight. A five-block cross shape is bundled for testing.

Legacy inspector widget classes and PySide1 brush shape classes cannot
execute automatically. Arbitrary plugin Python still has full local
privileges, so user opt-in and trusted source review remain necessary.

## 0.3.0a12 — Qt6 plugin panel callbacks

Explicitly loaded Python 3 plugins can now register native Qt6 panels
via `host.register_extension('widget', id, provider)` with provider(parent)
returning a QWidget. The host renders them in a managed closeable dialog.
Legacy PySide1 widget classes are not auto-instantiated or converted.

A built-in info panel is included as an example. This is an extension API,
not parity with MCEdit2's original dock manager and scenegraph previews.


## 0.3.0a15 — Selection block statistics without chunk rewriting

Use **Edit > Selection block statistics…** to count blocks in the current
bounded selection. In the options dialog, enable **Separate block-state
properties / numeric metadata** to distinguish properties such as facing,
rotation, or old Java numeric data. Otherwise counts are grouped by native
block identifier. Results show the total, number of distinct types, BE
locations (or **unknown** when the backend cannot report them), and the top
512 frequency rows. Selecting this action does not change or save the world.

The scan calls get_block and, where available, has_block_entity. It does
not deserialize and reserialize BE compounds and does not create a
transaction or write a region. Unreadable Sections abort the entire result.
The existing bounded selection size limit applies, and statistics are not
a replacement for real-world version-specific save/reopen validation.


## 0.3.0a16 — Filtered native block replacement with preview

With a bounded selection, choose **Edit > Filtered replace by native block
ID…**. Set a source ID / state and the replacement in the world's original
syntax. Leaving **Require exact properties / legacy numeric metadata**
unchecked matches *all* state variations for the source block ID. Checking
it replaces only the exact properties or legacy numeric data value. A preview
shows the total selection, matched blocks and proposed writes; the operation
requires a second explicit confirmation and becomes one undo step.

Block Entity positions are never silently overwritten or skipped. Any
proposed BE overwrite rejects the entire replacement before staging a
single change. If the backend cannot determine BE presence, filtered
writing is refused. Unrelated BE and modded NBT bytes remain untouched.
The original-version backend checks every new state before mutation, and
the filter re-scans at commit time. This tool does not fix lighting or
heightmaps and is not safe for concurrent game/server world writes; only
use offline disposable world copies during development.


## 0.3.0a17 — Original-format Python 3 plugin filter callbacks

Explicitly load a trusted plugin in Plugins > Manage Plugins. Registered
Python 3 filter callbacks appear under Plugins > Python 3 native block filters.
Choose the filter and destination native block ID/state; the editor previews
matches and conflicts before requesting an explicit confirmation.
The selection budget applies, and the edit is a single undo step.

A predicate has the signature predicate(record) -> bool, where record is an
immutable FilterBlock with position, original BlockState or numeric metadata,
and has_block_entity (bool, or None if unknown). Predicates receive no edit
transaction and are rerun before commit. Non-boolean results, unreadable
data, BE conflicts or missing backend BE queries reject the operation.

The bundled opt-in plugin includes native_stone_only as an example. Old
Python 2/PySide1 classes are never auto-executed. Third-party Python code
is trusted in-process, not sandboxed. Lighting/heightmaps, linked entities,
complex predicate options and complete old tool parity remain outstanding.


## 0.3.0a18 — Configurable filter panels

Trusted Python 3 plugins can register a named filter with typed options.
The bundled Configurable native block filter shows an original-format
source block, ID-vs-exact-state mode, minimum and maximum Y and an invert
toggle. The editor combines these settings and the destination block into
one Qt6 options dialog.

Previews stay read-only, and actual edits rescan with exactly the same
parameter values before one undoable commit. Unknown parameter names,
invalid numeric ranges, invalid choices and invalid block-state arguments
are refused. The filter never deliberately writes BEs; any protected
destination rejects the operation. Existing one-argument filter callbacks
remain compatible without migration.

No visual option editor is provided for unported Python 2/PySide1 plugins.
Third-party Python code still runs in-process with account permissions:
this API is not an isolation boundary. Real Minecraft modpack/world reload
testing remains outstanding; always use offline copies.


## 0.3.0a19 — Dockable MCEdit2 Tools panel

A right-side **MCEdit2 Tools** panel groups the existing editor actions
into one Qt6 toolbox. If hidden, restore it via **View > MCEdit2 Tools**;
Qt provides normal dock dragging and left/right docking. The operation
selector includes Paint and Select modes, native block-only selection edits
and any Python 3 tool, generator and filter plugins that you explicitly
loaded in the manager. Clicking **Activate / Run** delegates to the same
menu action or plugin transaction as before. It does not execute plugins
merely by scanning or opening the dock.

The block ID/state entry is synchronized with the top toolbar, and the
Selection Y min/max spinners also stay synchronized. **Refresh selection
info** shows the 3D selection volume and the safety budget. The 2D
viewport emits a selection change signal on mouse release. Select a region
first for actions that require it; generation is limited by the existing
selection budget. All editing remains in the original Java world format
and unrelated BE NBT is not rewritten.

This is an initial tool palette port, not complete MCEdit2 interface
parity. It does not supply arbitrary PySide1 panels, complex 3D model
overlays, cross-dimension transfer, Bedrock support or missing light fixes.
Offline disposable world copies are still required.


## 0.3.0a20 — Coordinate-first selection and contextual shape controls

The right-side **MCEdit2 Tools** dock now includes **Corner A X/Z** and
**Corner B X/Z** alongside the shared min/max Y spinners. Select
**Select XYZ corners** to define a 3D region without clicking blocks.
This is useful in skyblock/empty-air worlds: the selection is visual UI
state only, so no world chunks are loaded or created. The same bounds
appear in the slice and in the streaming 3D selection wireframe when
the 3D viewport is active. Oversized selections are rejected before
the existing selection is changed.

3D drag selection notifies the dock on mouse release and updates its
coordinates, while incomplete manually typed corner values are preserved
when editing only Y bounds. Switching worlds discards old selections.
The **Hollow selected volume** and **Sphere at view center** dock tools now
provide contextual thickness/radius and hollow style settings rather
than opening a separate generic tool dialog.

These tools delegate to the existing native EditSession and still refuse
unsafe BE overwrites or missing/invalid chunks. No version conversion,
lighting or heightmap generation occurs. The 3D renderer still does not
render every modded block model, and this is not yet a full MCEdit2 GUI port.
Use only backups or disposable offline worlds.


## 0.3.0a21 — Direct 3D selection handles

Choose **Select** and create a bounded region in the 3D OpenGL view
(or enter two XYZ corners in **MCEdit2 Tools**). Its wireframe now includes
six orange diamond-shaped face handles and a cyan center cross. Hold
**Alt + left mouse drag** on a face to resize that boundary. Drag the cyan
center with **Alt** to translate the region on X/Z; use
**Alt + Shift + left drag** for Y translation. The tool dock also includes
**Move X/Y/Z** and **Face handle / Face displacement** for exact integer
adjustments, and can scroll if the editor window is short.

These are selection-only UI operations: they do not move, edit, load or
create world blocks. Existing operation block limits and coordinate
validation are applied before accepting transformed bounds. Shrinking
a face stops at a one-block-wide selection. Actual block moves remain
a separate, explicit and undoable native editor operation; gizmo drags
are not world edits. The GL selection overlay is updated in both base
and streaming 3D views.

The gizmo handles are an initial functional pass, not complete original
MCEdit2 interaction parity. Full model rendering, lighting/heightmap
maintenance, Bedrock LevelDB editing and broad real-modpack fixtures
are not yet implemented/validated. Work only on disposable backups.


## 0.3.0a22 — Native Java world compatibility audit

Before changing a Minecraft map, use **File > Audit native world
compatibility (read-only)…**. Enter the maximum number of existing chunks
to inspect, default 64 and capped at 512. The report identifies the actual
region extension, level.dat and chunk DataVersions, counted Block Entities
and chunks parsed successfully versus damaged/unsupported chunks.
It also flags incompatible compression, inconsistent locations, malformed
Sections/BE metadata and .mca/.mcr mixtures. Reports explicitly say if
only a partial sample was checked; you can copy the result as plain text.

The scan reads only disk-backed Java region files; it does not create
terrain, edit metadata, touch session.lock, commit pending edits, or rewrite
unknown NBT. A passing scan is **NOT permission to save a valuable world**.
Game-dependent lighting, heightmaps, block updates, entities, modded state
relationships and full-world behavior are not certified; use disposable
offline backups. The existing original-format writer still refuses unsupported
operations and does not silently use an Amulet DataFixer migration.


## 0.3.0a23 — Saving with known lighting / heightmap limitations

Before saving a native Java world, open **File > Inspect pending lighting /
heightmap impact (read-only)…**. It lists edited X/Z columns, chunk
coordinates, section heights, existing light arrays / heightmaps, and
scanning limitations. Saving displays an explicit reminder that the old
derived values may now be out of date and requires user confirmation.
The preview is read-only and does not modify world or editor data.

The native writer now checks that original lighting, heightmaps and
associated metadata tags are *byte-identical* across the pending chunk
patch. This prevents accidental overwrites, but preserving metadata is
not equivalent to recomputing it. At this stage there is NO general
Minecraft lighting solver, no modded light/opacity model, and no correct
cross-version heightmap updater. Use the editor only on disposable offline
copies and check the modified region in the actual Minecraft version and
mods before relying on it.


## 0.3.0a24 — Inspect WORLD_SURFACE heightmap candidates (without saving)

Choose **File > Preview WORLD_SURFACE heightmap (memory only)…** while
a chunk under the view center has staged edits. The editor asks for
**minimum world Y** and **total build height** (not the maximum Y).
Verify these values for the actual dimension, especially if a datapack
changes build limits. Typical vanilla 1.18+ Overworld uses min Y -64,
height 384; older Java Overworld commonly uses 0 and 256. These suggestions
are never used as proof of a custom world's geometry.

For up to 16 changed X/Z columns, the preview parses the original
WORLD_SURFACE long array in the native version's format and checks its
current values against untouched blocks. It then builds a candidate
highest-non-air map, refusing unknown mod namespaces, missing/invalid
array layouts or stale originals. It validates that no other NBT bytes
would change. The resulting preview is **not applied to the world**, and
there is no button to commit it.

This is useful for validating serialization/geometry work and identifying
why a heightmap cannot yet be safely rebuilt. It does NOT implement
general heightmap updates, motion-blocking rules, block light, sky light,
fluid physics or automatic game relighting. Normal Save continues to
preserve original derived metadata and warns that it may become stale.
Use only offline disposable world copies.


## 0.3.0a25 — Optional WORLD_SURFACE write for offline test worlds

**Default Save remains unchanged**: preserve the original light and
heightmap NBT byte-for-byte, even if its values may be stale.

For experimental test-world copies, first use the memory-only WORLD_SURFACE
preview for the current chunk. Next choose **File > Approve WORLD_SURFACE
update for current chunk (experimental)…**, verify the exact dimension
minimum Y and full build height, read the warning and explicitly approve.
Every block-edited chunk must have its own approval, or Save will refuse
rather than silently mix approved/unapproved block changes. At most
16 chunks / 32 MiB of source plus candidate NBT may be approved at once.
**File > Cancel pending WORLD_SURFACE approvals** discards all approvals
without changing any world or pending block edits.

After approval, Save asks for a second confirmation. The native writer
rechecks the exact old chunk, every staged edit and independently rebuilt
candidate during its existing preflight/backup/rollback transaction. Only
the approved WORLD_SURFACE NBT long array is allowed to differ from the
original metadata. Other heightmaps, lighting, Block Entities and unknown
modded NBT remain protected. Any mismatch aborts the whole save before
region replacement.

The opt-in feature does **not** mean a modern Minecraft world has
consistent lighting, collision heightmaps, physics, modded data or other
dependent structures after edits. Do not enable this option for
valuable worlds. Use offline disposable copies, inspect backed-up files,
and test in the actual game/modpack. The normal editor remains experimental.


## 0.3.0a26 — MOTION_BLOCKING and OCEAN_FLOOR previews

For experimental **Java 1.21 / 1.21.1** region files only, use
**File > Preview MOTION_BLOCKING / OCEAN_FLOOR (read-only)…**.
Select the desired native heightmap, verify exact dimension minimum Y
and full build height, and explicitly confirm the world's relevant
vanilla heightmap tags have not been overridden by mods or datapacks.
A bounded preview of at most 16 changed X/Z columns validates original
heightmap values against original blocks and previews the chosen edit.
This is a report, NOT an option to save the candidate. The default
No confirmation protects against making unstated assumptions.

Unlike WORLD_SURFACE, MOTION_BLOCKING includes some fluid blocks while
OCEAN_FLOOR does not. A small explicit vanilla state subset is supported:
air variants, selected simple solid cubes, and water/lava with ordinary
fluid levels. Slabs, leaves, stairs, waterlogging, complex collision,
modded blocks and other versions are currently refused. This is
deliberately narrower than real Minecraft's available blocks. The
currently implemented optional WORLD_SURFACE save approval does not
enable saving either new map. Real modded registry rules, more complex
block states, extra heightmap types and all relighting remain pending.


## 0.3.0a27 — Import exact collision heightmap facts as JSON

Choose **File > Preview collision heightmap using state evidence JSON…**
to select a version-bound UTF-8 state-rules file for read-only
MOTION_BLOCKING or OCEAN_FLOOR previews. The JSON supplies the exact
DataVersion, provenance claim, each precise block-state name/properties
and two boolean facts: membership in blocks_motion_in_heightmap and
whether that state contains fluid. This permits previewing explicitly
enumerated waterlogged, stair, slab, leaf and modded block states that
the tiny built-in vanilla allowlist refuses; any state absent from the
file still fails. No dynamic collision engine is invented.

The user must verify that the evidence describes the *actual* Minecraft
game/mods/datapacks used for the world. JSON metadata alone does not
authenticate or verify runtime Minecraft tags or modded block properties.
The editor shows a provenance summary and fingerprint and asks for
explicit confirmation. The importer executes no scripts or plugins.
A mismatch, missing fact or invalid JSON refuses the preview.
The result is NEVER written, and regular/approved WORLD_SURFACE saving
continues unchanged. Consult doc/heightmap_evidence.md for a schema
example; real-game registry extraction remains future work.


## 0.3.0a28 — Capture precise rules from a running NeoForge 1.21.1 game

An auxiliary, optional NeoForge 1.21.1 mod source overlay now exists in
`tools/neoforge-1.21.1-exporter/`. It compiles against the official
ModDevGradle MDK using Java 21 (independent CI build verified) but is
**not yet confirmed by an actual game-session export**. Its operator-only
`/mcedit2export` command is intended to write a read-only registry
snapshot into the game directory under `mcedit2-evidence/`, without
modifying saves. The user then selects that JSON through the existing
**File > Preview collision heightmap using state evidence JSON…** menu.

This path queries the game's real heightmap predicate and FluidState for
every exact block state, including installed mods; it does not infer
collision rules from block names. The historical JSON key
`blocks_motion_in_heightmap` denotes the OCEAN_FLOOR predicate, **not a
Minecraft block tag**. The expanded bounded importer accepts up to
200,000 states and 32 MiB. If an export exceeds these limits it must
fail, not truncate.

The helper is optional and does not enable collision heightmap saves
or light rebuilding. Do not place its unverified runtime build on a
valuable production server; test in an isolated modpack instance first.


## 0.3.0a29 — Game-tested evidence export and real 1.21.1 read-only world parse

The optional NeoForge helper is now verified in a disposable actual
Minecraft 1.21.1 server, not merely compiled. An isolated CI server
started, exported 26,684 exact registered states (DataVersion 3955),
and had its JSON verified by Python for digest, source/format and
representative states. A double-gated CI-only startup hook lets this
happen without requiring human console input; normal server installs
still export ONLY after an operator runs /mcedit2export.

The same test server saved a genuine world on shutdown. MCEdit2's
native Java region reader performed a **read-only 48-chunk scan** over
four .mca files, checking 168 indexed entries in this sample and
confirming level.dat/region-file SHA-256 equality before and after.
This is the first CI-backed acceptance check using actual
Minecraft-generated Java 1.21.1 region files rather than synthetic
unit-test fixtures.

For reproducibility, see
tools/neoforge-1.21.1-exporter/verify_runtime_export.py,
verify_runtime_world.py, and the separately triggered
.github/workflows/neoforge-exporter-compile.yml.

Do not confuse read-only compatibility with safe editing and saving:
vanilla and especially modded gameplay state, BE mutations, relighting,
heightmap updates and reopen behavior still need independent tests.

## 0.3.0a30 — Original MCEdit2 tool and panel migration batch

This implementation-first batch adds several original MCEdit2 workflows
to the Python 3 + Qt6 editor. These are **new actual modules**, not a
Python2/PySide compatibility shim, and do not silently convert any
world version or rewrite external NBT.

- **View > MCEdit2 World Panels**: World Info data and raw NBT;
  singleplayer/playerdata browsing, inventory and EnderItems slots with
  inspectable opaque ItemStack NBT; selected chunk metadata and native
  chunk NBT; filled map_N.dat browsing with legacy color-table pixels.
  Every feature in this dock is **read-only**. Unknown modern map colors
  use a clearly visible diagnostic color; they are not assumed correct.
- **Browse blocks…** next to the toolbar's BlockState field: search
  states observed in one existing original chunk. Optionally open the
  runtime registry JSON generated by the earlier NeoForge helper to
  browse modded states. This changes **only the block selection input**,
  not world data. Unsupported block versions are not DataFixed.
- **View > MCEdit2 Pending Imports** or **File > Queue structures for later
  placement…**: queue .nbt, .schem or .schematic, inspect and change XYZ
  placement, then apply all through a single preflight and undo record.
  Structures with unsafe BE/entity payloads are still rejected by the
  original-format loaders. No automatically upgrading block states.
- **Edit > Original shape brushes (fill / replace)…**: square, round
  ellipsoid, diamond, vertical cylinder and parabolic dome with size,
  center, optional hollow shell and fill/replace mode. Operations are
  bounded by EditSession and use existing Undo and BE guards.
- **View > MCEdit2 History**: inspect the current session's Undo/Redo
  records and invoke the same Undo/Redo actions. Does not invent
  persistent history or change the backed-up original map files.
- **View > MCEdit2 Chunk Minimap**: displays the index of generated
  chunks from the original .mca/.mcr region headers, supports click
  navigation and mouse-wheel zoom. It neither decompresses all sections
  nor creates any map data during navigation.

The features above are only one large **partial migration batch**, not
a complete original MCEdit2 port. Original writable player/world/map
panels, richer inventory components, all modern map colors, block model
rendering and shader parity, graphical schematic overlay handles,
Bedrock LevelDB, version-specific lighting/biome updates, legacy plugin
GUI APIs and modded entity movement remain outstanding. Do not try them
on a valuable saved world.

The user requested migration progress ahead of a full test campaign.
Accordingly, these new features have **not yet had final CI/game-save
acceptance**; the most recent previous green CI validates only earlier
a29 state. Treat a30 as experimental source development, not release.


## 0.3.0a31 — Original-style resource-pack block model meshes

The streaming Qt6/OpenGL editor now has an integrated partial JSON
block-model pipeline beyond colored full cubes. The new
`block_models.py` resolves model parent inheritance, texture variables,
variant selection and multipart OR/AND conditions; maps bounded model
element coordinates, faces and UVs; and applies orthogonal blockstate
rotations plus supported element pivots and ±22.5°/45° rotations.
Geometry such as stairs, slabs, fences, thin planes and other regular
vanilla JSON multi-cuboid models can enter the existing per-Section mesh
pipeline. This is based on the resource JAR/ZIP files explicitly added
to the editor; **it does not run Minecraft game registries or mod code**.

Ordinary full cubes retain background greedy face merging. Special
element faces are resolved to stable atlas tile indices on the UI
thread and meshed on the existing worker pool in the identical VBO
layout. A Section's immediate neighbors participate in face-culling
decisions. The shader now discards fully transparent PNG texels to
permit alpha-cutout details without rendering opaque rectangles.

Known remaining limitations: only the supported vanilla JSON geometry
subset (unsupported models fall back to diagnostic cubes); full
Minecraft tint/color maps, animated atlas images, weighted random
alternative selection, water/glass translucent sorting/blending,
entity models, external mod-specific model loaders, lighting/shadows
and exact geometry-based picking are still incomplete.

**This source migration has not yet received a new full CI/screenshot
validation run**, following the user request to prioritize completing
unmigrated features first. Do not treat it as production-ready.


## 0.3.0a32 — Separate opaque and translucent OpenGL layers

The resource-backed 3D streaming viewport now draws opaque and blended
layers separately rather than writing every textured face to the same
opaque depth buffer. Known vanilla water/glass and states whose atlas
textures contain partial alpha enter the blend layer. Alpha-cutout
foliage remains in the ordinary depth-writing layer. Special geometry
from the previous `block_models.py` pipeline also participates in
layer selection.

The Qt atlas now copies PNG alpha into tiles directly; the greedy mesh
worker accepts an opt-in layered result consisting of opaque and
translucent byte arrays. The UI combines both in one Section VBO
without altering native block records. Per frame, opaque geometry
renders first; translucent Sections render from far to near with blending
enabled and depth writing disabled. A **View > 3D translucent opacity…**
action stores the chosen setting and changes only the render uniform.

The implementation remains an approximation: cross-Section sorting
does not properly resolve within-Section glass crossings or fluid
surfaces. Precise per-face/fragment ordering, Minecraft water-level
meshes, biome tint and sophisticated transparency effects remain to be
ported. These a32 changes have NOT yet undergone full CI or screenshot
acceptance, following the user preference for feature migration before
a broad test campaign. This is experimental code, not a release.

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
