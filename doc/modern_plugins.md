# MCEdit2 Next plugin API (Python 3 / Qt6)

The original MCEdit2 plugins used a dynamic Python module loader, registered
command/tool/generate/inspector classes and offered a per-editor-session
interface. MCEdit2 Next preserves the core concepts but uses explicit opt-in
installation, a stable manifest and the native Anvil editing transaction layer.

## Installing and enabling

1. Put a plugin directory into `~/.mcedit2-next/plugins/`.
2. Each directory requires `plugin.toml` and `__init__.py`.
3. In the editor choose **Plugins > Manage Plugins… > Rescan**, select the
   plugin, then **Load** and explicitly confirm that you trust the code.
4. Commands appear under the Plugins menu (and their optional submenus).
5. **Unload** or **Reload** removes old registration entries.

On Windows, `~` refers to your user profile folder. Scanning reads TOML
metadata and does not import plugins. Loading can execute arbitrary Python
code with the user's account privileges. **There is no sandbox**: installing
an untrusted Python plugin is equivalent to running an untrusted Python script.

The included sample at
`modern/mcedit2_next/builtin_plugins/selection_tools` provides commands to
count/replace selected blocks. Built-in examples are also disabled by default.

## Manifest

```toml
[plugin]
id = "mycompany.example"
name = "My Example"
version = "1.0.0"
api = 1
description = "Adds one command to the Plugins menu"
entrypoint = "__init__.py"
```

IDs must be lowercase names of 3–80 characters, including the characters
`a-z`, `0-9`, `.`, `-` and `_`. The current API version is **1**.
Plugin entrypoints may be `__init__.py` or a simple filename in the plugin
directory. Folder symlinks and path traversal entrypoints are rejected.

## New command API

```python
def register(host):
    def paint_corner(ctx, options):
        if ctx.selection is None:
            raise ValueError("Select an area first")
        x, y, z = ctx.selection.minimum
        ctx.set_block(x, y, z, ctx.make_block(options["block"]))

    host.register_command(
        "paint_corner",
        "Paint selection corner",
        paint_corner,
        options=[
            {"type": "blocktype", "name": "block",
             "text": "Block", "value": "minecraft:stone"}
        ],
        submenu="Examples",
    )
```

Commands receive a `PluginContext` providing `selection`, `get_block`,
`set_block`, `make_block`, `fill`, `replace` and read-only
`get_block_entity_raw(x, y, z)` (returns bytes or None). Selection is a
three-dimensional inclusive `Box` with `minimum`, `maximum`,
`volume`, `size` and `points()`. In the current GUI, dragging selects X/Z
and the Selection Y min/max spin boxes choose the vertical range.

**Undo guarantees:** Plugin block writes are staged until the callback
returns successfully, then committed as **one undo entry**. A callback error
before commit discards pending block writes. Writes into a BE position are
rejected by the native backend. The editor limits each operation to 32768
changed blocks by default; selection traversal should also be bounded.

The parameter form currently supports `label`, `int`, `float`, `bool`,
`text`, `choice` and `blocktype`. Blocktype values are converted from names
and states into a version-neutral BlockState before the callback.

## Porting MCEdit2 SimpleCommandPlugin

Existing MCEdit2 SimpleCommandPlugin classes can often preserve their
`perform(dimension, selection, options)` logic:

```python
from mcedit2_next.plugins import registerPluginCommand, SimpleCommandPlugin

@registerPluginCommand
class ReplaceStone(SimpleCommandPlugin):
    displayName = "Replace Stone"
    options = [
        {"type": "blocktype", "name": "target",
         "value": "minecraft:glass", "text": "Replace with"}
    ]

    def perform(self, dimension, selection, options):
        stone = dimension.blocktypes["minecraft:stone"]
        for x, y, z in selection.positions:
            if dimension.getBlock(x, y, z) == stone:
                dimension.setBlock(x, y, z, options["target"])
```

This preserves `blocktypes[name]`, `dimension.getBlock`,
`dimension.setBlock` and `selection.positions`.

**Not binary/source compatible by default:** Original modules importing
`PySide`, `mcedit2.plugins`, `mcedit2.editortools` or old
`mceditlib` classes still require a port. We do **not** monkey-patch
`sys.modules["mcedit2.plugins"]` or execute legacy Python 2 bytecode.
Legacy `CommandPlugin` constructors and `addMenuItem` need refactoring
to the new command registration API.

## Extension categories

```python
def register(host):
    host.register_extension("tool", "my_editor_tool", MyToolClass)
    host.register_extension("brush_mode", "my_brush", MyBrushMode)
    host.register_extension("brush_shape", "my_shape", MyShape)
    host.register_extension("inspector", "my_inspector", MyInspector)
    host.register_extension("tile_entity", "my_machine", MyMachineDecoder)
    host.register_extension("generate", "my_generator", MyGenerator)
    host.register_extension("widget", "my_panel", MyWidget)
```

These categories support load/unload/reload without retaining orphaned classes.
**Brush modes are connected to both the Qt6 horizontal-slice and basic OpenGL 3D views.** A brush
mode provider is a callable taking `(ctx, point)` and using `ctx.set_block`
to stage edits. Select it under **Plugins > Brush modes**. An entire mouse
drag is committed as a single undo operation. The example plugin registers
a checkerboard brush mode. The other categories are registration-only until
the corresponding GUI components are ported. Legacy decorators `registerCustomWidget`, `registerToolClass`,
`registerGeneratePlugin`, `registerBlockInspectorWidget` and
`registerTileEntityRefClass` are provided for class **registration** only.
They do not make old PySide 1 tools or inspectors executable in Qt6, nor do
they automatically add editors, NBT support or UI panels.

Future API extensions should use versioned manifests and capability-specific
contexts. Plugin hooks for BE data must maintain the native backend's
raw-NBT preservation guarantees, must not silently normalize unknown tags,
and should allow read-only inspectors.

## Limitations

- An editable backend is currently implemented only for Java paletted
  worlds (1.13+ in supported Anvil formats).
- An experimental 3D OpenGL voxel viewport is available alongside the 2D slice. It supports basic full-cube resource-pack/JAR textures but lacks complex or rotated models, unlike the original MCEdit2 renderer.
- The plugin mechanism is trusted in-process execution; cooperative API
  restrictions are **not** a security boundary.
- If a plugin directly accesses disk/world files using ordinary Python APIs
  outside PluginContext, the editor cannot track, undo or roll back its edits.
  Only use plugins from sources you trust.
- Use **Edit > Inspect BE at selection origin** to browse the untouched
  binary NBT in a read-only Qt6 tree dialog. Modded nested fields are shown
  without converting them to another game's block entity schema.
- BE-safe staged edits do not yet support deliberate BE NBT modifications,
  entity editing, or preserve cross-block semantics when moving tile entities.

## Block entity snapshots and experimental clipboard

The editor can explicitly copy Java 1.13+ Block Entities as opaque NBT
payloads through **Edit > Copy with BE NBT (experimental)**. Pasting with
the matching BE-aware command relocates only root TAG_Int `x`, `y`, and
`z` coordinates, preserving every other unknown tag's original bytes.

This is not a general-purpose block-entity schema converter, a modded
multiblock copier, or a capability/UUID translator. It refuses existing
BE targets and pre-1.13 BE transfers. Plugin authors should not bypass
the native backend protection or write raw NBT directly: such writes
are not covered by the editor transaction and rollback guarantees.

## Native Block Entity decoder callback (experimental)

Opt-in Python 3 plugins can register a decoder keyed by the exact `id` string
stored in the original BE NBT root compound (for example `minecraft:chest`
or `modid:machine`):

```python
def register(host):
    def describe(record):
        # record.position: (x, y, z)
        # record.state: original native block state
        # record.block_entity: original, unmodified NBT compound payload bytes
        return {"NBT size": len(record.block_entity), "State": str(record.state)}

    host.register_tile_entity_decoder("modid:machine", describe)
```

After explicitly loading the plugin, use **Plugins > Python 3 Block Entity
decoders**. The view is read-only. MCEdit2 Next parses only the BE root
`id` to verify identity; it passes the complete original payload to the
callback without normalization. A wrong ID, absent BE, malformed NBT, deleted
plugin, legacy PySide1 reference class or unsupported callback output is
rejected. Callbacks return text or up to 100 string-keyed scalar fields
(the displayed result is capped at 16,000 characters). A basic
`minecraft:chest` entry-count decoder is supplied by the optional
`selection_tools` example plugin.

Legacy `registerTileEntityRefClass` remains registration-only. This does
not enable arbitrary editing of modded machine inventories/links: untrusted
plugins still execute normal Python in-process and **are not sandboxed**.


## Original-format Python 3 filter predicates (0.3.0a17)

Register an explicit Python 3 predicate via
host.register_extension("filter", "stone_native", filter_stone).
The callback receives a frozen FilterBlock snapshot with position,
state (the world-native BlockState or legacy numeric metadata), and
has_block_entity (bool or None if the backend cannot check). It must return
a real bool and never edit world files. For example:

    def register(host):
        def filter_stone(record):
            block_id = getattr(record.state, "name",
                               str(record.state).split("[", 1)[0])
            return block_id in ("minecraft:stone", "mcedit2:legacy_id_1")
        host.register_extension("filter", "stone_native", filter_stone)

Loaded predicates appear under Plugins > Python 3 native block filters.
PluginManager.preview_filter(name, session, selection, target) computes an
unchanged-world FilterPreview; PluginManager.apply_filter(...) rescans and
commits one guarded native edit. The GUI confirms the preview first.
Any BE overwrite, missing presence API, unloaded provider, unsupported state,
nonboolean predicate or unreadable Section refuses the operation.

Unported Python 2/PySide1 classes are never run implicitly. Python plugins
have full local user permissions: this API is NOT a security sandbox.
Test only on disposable offline world copies.


## Typed configurable filters (0.3.0a18)

For a configured filter, use PluginHost.register_filter rather than the
single-record register_extension filter call. The Python 3 callback receives
(record, options) and options always contains validated values:

    def register(host):
        def match(record, options):
            actual = record.state
            wanted = options["source"]
            if options["match_mode"] == "exact":
                return actual == wanted
            return getattr(actual, "name", str(actual).split("[", 1)[0]) == (
                getattr(wanted, "name", str(wanted).split("[", 1)[0]))
        host.register_filter("native_filter", "Original block filter", match, [
            {"type": "blocktype", "name": "source", "value": "minecraft:stone"},
            {"type": "choice", "name": "match_mode", "value": "id",
             "choices": [("ID", "id"), ("Exact state", "exact")]},
        ])

The Qt6 dialog adds a separate reserved native destination parameter and
supports blocktype, choice, int, float, bool, text and label widgets.
Provided options may omit fields with valid defaults. Values are checked
for type, range and allowed choices, and block states are parsed using the
current world's native block parser. The reserved name
__mcedit2_destination must not be declared by a plugin.

PluginManager.preview_filter and apply_filter accept optional options
as a final argument. The latter revalidates all parameters, reruns the
predicate and checks BE conflicts before committing. Old single-argument
register_extension filter callbacks remain supported without options.
This is for trusted scripts only, not a sandbox.


## Dockable tool palette (0.3.0a19)

After explicitly loading a trusted Python 3 plugin, its runnable native
tool, generator and filter extensions also appear in the right-hand
**MCEdit2 Tools** dock's operation selector. The selection is shared
with 2D/3D editor views and the top toolbar. Selecting a command and
clicking **Activate / Run** follows the existing plugin manager callback
and transaction path; no new direct world-writing plugin API was added.

When a plugin is unloaded, its dock entry disappears. Reloading updates
registration without leaving a stale action available to execute.
The dock shows already loaded providers only and does not import any
discovered plugin by itself. Legacy PySide1 classes still need an explicit
Python 3 port; running Python code from a plugin is not sandboxed.


## XYZ selection and contextual native tools (0.3.0a20)

The dock can establish a bounded selection from explicit XYZ coordinates,
even in an empty world where 3D ray casting cannot hit a solid block.
This is selection state only and does not call any plugin or write to disk.
Plugin tools and generators still receive the selected native Box through
their existing context and are subject to the core block-operation budget
and BE guards. Hollow shell thickness and sphere radius are Qt6 dock options
executed using the same original-format EditSession commands; this release
adds no new plugin privilege or direct chunk mutation API.


## Read-only compatibility audits (0.3.0a22)

A NativeAnvilWorld instance offers compatibility_audit(max_chunks=64,
max_regions=32). This samples existing Java region headers/chunks and returns
an immutable WorldAudit summary with readable, damaged, unsupported,
Block Entity and original-format counts; format_report() creates human
readable text. It is a diagnostic function, not a plugin execution primitive.
It does not load arbitrary user plugins, change selection/edit state or
generate/cross-convert world chunks. Passing an audit does not guarantee
modded or lighting/heightmap compatibility.


## Pending block-edit derived metadata impact (0.3.0a23)

NativeAnvilWorld.derived_data_impact(max_chunks=128) provides a bounded
read-only report of pending block-state changes, modified X/Z columns,
section heights and original heightmap/light tag presence, without
calling plugin callbacks, changing edit transactions or modifying chunk
data. The maximum inspected chunks is 512; reports mark partial samples.
The native writer also independently verifies byte-for-byte preservation
of derived NBT metadata during save preparation, aborting on any
unexpected mutation. The API deliberately does **not** recalculate
Minecraft or mod-specific lighting/heightmaps.


## Heightmap codecs and no-write preview (0.3.0a24)

The native heightmap_codec module has explicit-layout
HeightmapLayout(min_y,height,data_version), decode_values/encode_values
for 256 column offsets and preview_world_surface for up to 16 edited
columns. NativeAnvilWorld.preview_world_surface(chunk_x,chunk_z,
min_y=...,height=...) builds a candidate from staged changes, reads the
original chunk and returns immutable preview/report data. **It neither
changes world caches/edit state nor commits any candidate.** The current
native writer deliberately leaves pre-existing light/heightmap metadata
unchanged and still applies its byte-preservation guards. Heightmap
packing does not justify guessing unknown modded isAir, collision,
fluid or light semantics.


## Explicit WORLD_SURFACE save approval (0.3.0a25)

NativeAnvilWorld.approve_world_surface_update(cx,cz,min_y=...,height=...)
accepts a strictly verified in-memory candidate for one Java 1.13+ chunk,
and approved_world_surface_chunks reports pending approvals.
cancel_world_surface_updates() clears them without altering the world.
Approvals are bounded (16 chunks / 32 MiB source plus candidate data),
tied to the exact original chunk bytes and complete pending block edits,
and revalidated independently when Save runs. Native Save refuses missing
or stale approvals and permits a byte change ONLY to the approved
WORLD_SURFACE long-array tag while preserving other derived metadata and
modded NBT in the original region backup/rollback transaction.

Approval is a separate, explicit user operation. Neither the original
read-only preview nor plugin loading auto-enables heightmap writes.
This does not recalculate other heightmaps, light, unknown modded
semantics or guarantee production-world compatibility.


## Read-only collision heightmap classification (0.3.0a26)

heightmap_predicates.classify_heightmap_state(state,name,data_version)
strictly classifies a tiny enumerated vanilla Minecraft 1.21/1.21.1
subset for MOTION_BLOCKING or OCEAN_FLOOR. The predicate uses no plugin
code and never invents dynamic block tags; unknown states raise
HeightmapError. NativeAnvilWorld.preview_collision_heightmap(cx,cz,
name=...,min_y=...,height=...,vanilla_tags_confirmed=True) returns a
bounded, read-only SurfacePreview for an original existing heightmap,
requiring explicit user assumptions about vanilla unmodified tags.
DataVersions are limited to 3953 and 3955. It does not add approvals,
generate world chunks, touch chunk caches or patch any region files.
The WORLD_SURFACE opt-in writer does not accept these heightmap names.


## Version-bound state evidence for collision maps (0.3.0a27)

heightmap_evidence.parse_heightmap_evidence(bytes) and
read_heightmap_evidence(path) validate strict UTF-8 JSON schema 1 and
return immutable HeightmapEvidence facts keyed by canonical exact
BlockState strings. The JSON names a single modern Java DataVersion,
a source claim and exact blocks_motion_in_heightmap / has_fluid
booleans. Limits: 4 MiB, 20,000 states. Unlisted states, wrong version,
duplicate keys, odd properties and invalid shapes fail closed.
The optional evidence= parameter to classify_heightmap_state,
preview_collision_heightmap and NativeAnvilWorld.preview_collision_heightmap
uses the imported rules for **read-only** heightmap computations.
The loader never executes Python, auto-loads mods or authenticates
third-party registry claims. The caller must explicitly acknowledge
that the snapshot matches its Minecraft installation. No additional
heightmap writes or approvals are introduced.


## Optional game runtime evidence bridge (0.3.0a28)

The source-only NeoForge 1.21.1 helper can capture
`Heightmap.Types.OCEAN_FLOOR.isOpaque().test(state)`,
`Heightmap.Types.MOTION_BLOCKING.isOpaque().test(state)` and
`!state.getFluidState().isEmpty()` for exact canonical block states.
It verifies the OR invariant before exporting JSON schema 1, and is
compiled separately from the Python/Qt backend using an optional
official-MDK CI job. No in-game end-to-end result has been verified yet.

The JSON importer accepts at most 32 MiB and 200,000 exact states.
The legacy `blocks_motion_in_heightmap` field records the effective
OCEAN_FLOOR predicate rather than any asserted block-tag membership.
This runtime bridge grants no writer privilege or plugin execution
rights; MOTION_BLOCKING/OCEAN_FLOOR candidates remain read-only.


## Runtime evidence and real region smoke CI (0.3.0a29)

The optional Java 21/NeoForge 1.21.1 helper now has an independently
triggered real server runtime smoke test. Behind two CI-only conditions
it exports a real DataVersion-3955 snapshot (26,684 states in the
first successful run), which the Python reader validates via its exact
SHA-256/content protocol. The same CI probes 48 genuine Minecraft
1.21.1 world chunks through NativeAnvilWorld.compatibility_audit, asserts
no unsafe/damaged scanned entries and checks source file hashes.
No user-world changes, artifact publication or plugin execution result.

The exported evidence remains only a source for read-only collision
heightmap previews. No production-mode automatic export and no additional
heightmap writer privilege are added.
