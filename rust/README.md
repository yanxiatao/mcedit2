# MCEdit2 Rust + iced workspace

The root of this branch is the native rewrite, not the original Python application. The old implementation is preserved in the repository's `master` history and in the separate Python 3 Draft PR. This workspace contains no Python runtime, Cython or Qt dependency.

## Crates

| Crate | Purpose |
| --- | --- |
| `mcedit2-world` | Bounds-checked Java Minecraft world/NBT/region formats, original-state editing, transaction and schematic helpers |
| `mcedit2-iced` | iced 0.14 desktop, wgpu 3D view and selection, streaming/chunk meshing, vanilla resource-pack models and multi-atlas textures |
| `mcedit2-cli` | Headless source world inspection |

## Resource-pack overlay order

The native texture input supports multiple local folders, ZIP resource packs
or client JAR paths separated by `;`, **lowest priority first**. The selected world's `saves/` parent determines its Minecraft game
directory. The renderer attempts to layer a matching vanilla client JAR,
resource-bearing JARs under that same instance's `mods/`, generated
`kubejs/assets/` files, and the `file/...` packs enabled in that
instance's `options.txt`, in increasing priority. Only explicitly selected
packs are enabled; other installed resource packs are ignored. Manually
selected ZIP/JAR/folder layers have the highest priority. Upper layers override
only matching PNG, model JSON or blockstate JSON identities; absent overrides
retain an intact lower layer.
A new static PNG does not inherit stale `.mcmeta` animation instructions
from a lower layer. All decoding happens in the existing background worker,
and the combined atlas is generated once under existing memory/GPU budgets.

## Dotted mod resource identities and asset roots

Minecraft Java namespaces and resource paths may contain periods (for example
`create.addon:block/machine/casing.v2`). The native PNG and JSON readers
now share strict canonical validation for these IDs, while rejecting invalid
case, backslashes, empty or dot-only directory segments, and traversal
components. Directory resource packs refuse an `assets/` symlink and ignore
symlinked children; loading remains read-only. Directory iteration is
deterministic even if the OS returns entries in a different order.

## Development commands

From `rust/`:

```sh
cargo check --workspace
cargo test --workspace
cargo run -p mcedit2-iced
cargo run -p mcedit2-cli -- "/path/to/extracted/world"
```

Windows packaging is an opt-in workflow. Do **not** trigger Actions, tests, builds or packaging on the GitHub branch merely to validate source changes unless the user expressly requests them.

## Current implementation

- Accept original Java Minecraft world folders, metadata, signed coordinates and known Alpha, MCRegion and Anvil source layouts. Unknown/modded data must not be invented or silently rewritten.
- Open directly into a 3D spectator-style viewport; fly, mouse-look, inspect, select and stream existing chunks. Sparse distant LOD is a visualization only, never an editing authority.
- Auto-discover Java PNG, `.mcmeta`, blockstate JSON and model JSON assets from the selected world's vanilla client, installed Mods and enabled resource packs. Mixed-resolution, multi-page GPU atlas packing preserves per-face page and UV mapping. Unresolved foreign-namespace models display the explicit purple/black missing texture, not inferred block geometry or name-based sprites.
- Use in-memory edit groups, preview/undo, native source-validation and deliberately guarded save paths. World writes must not occur implicitly from inspection or a resource-pack load.

Read [COMPATIBILITY.md](COMPATIBILITY.md) for the detailed data-version/storage safety policy. See [original-format regression fixtures](../test_files/README.md) and `crates/mcedit2-world/tests/` for native regression coverage.

## Migration status

This is an active rewrite, not complete MCEdit2 feature parity. Old Python sources, Sphinx docs, PySide/Cython setup and Python-only benchmarks/tests are intentionally absent from **this branch**. They remain available in the original branch/history. Do not use the old Python app instructions for building the Rust binary.

The latest code must be checked with the toolchain before a distributable build is described as verified. The previous Windows development ZIP does not automatically contain newer source changes.

## Rendering architecture: Amulet reference and source-driven visibility

The 3D renderer is being brought closer to the chunk lifecycle used by
[Amulet Map Editor's chunk builder](https://github.com/Amulet-Team/Amulet-Map-Editor/blob/0.10/amulet_map_editor/api/opengl/mesh/level/chunk/chunk_builder.py)
and [region merger](https://github.com/Amulet-Team/Amulet-Map-Editor/blob/0.10/amulet_map_editor/api/opengl/mesh/level/region.py).
These are architectural references, not copied code or proof of parity.

- No fixed global spectator face or visible-voxel count truncates readable
  original source geometry. Requested source chunks are shown completely when
  their mesh is available; no geometry is fabricated to fill absent chunks.
- Exact chunk vertices are cached against exact source voxel values, model
  style and source origin. A cache entry is retired when its chunk leaves the
  current source snapshot (or its source/style changes), rather than after an
  arbitrary fixed number of cached vertices or chunks.
- Source stream snapshots coalesce: same-world chunk batches do not repeatedly
  cancel the currently active geometry build, which prevented any completed
  viewport from appearing when IO was faster than geometry creation.
- Large meshes are segmented on complete six-vertex faces at the active
  adapter's `max_buffer_size` and the draw API's index bounds. All segments
  are uploaded and rendered; the segmentation is not a face-count budget.
- Offscreen color/depth targets use `max_texture_dimension_2d` from the
  active wgpu device, not a fixed 4096px threshold. A 128-chunk streaming
  radius is now selectable, while the planner does not impose a hidden 64
  chunk restriction.

Resource decompression, image sizes and GPU texture-array limits still need
validation against memory/device capabilities; these are not permission to
discard successfully read world chunks. The existing mixed-resolution atlas
memory guard is still present and may downsample high-resolution textures in
large packs. Improving multi-array atlas scalability remains separate work.

The above Rust tests are source-level regressions, not a verified Windows
GPU result. A full local `cargo check --workspace`, `cargo test --workspace`,
and a large authentic Modded-world visual comparison against Amulet are
still required before describing renderer parity.

## Camera-facing full-ring streaming

The native spectator still schedules **every** actual Java chunk inside the
selected radius and keeps the exact nearby voxel sources separate from
read-only sampled distant LOD. Within each ring, the streaming worker now
prefers the chunks in front of the camera before those behind it. The first
distant warmup uses the same priority, so the visible horizon can appear
earlier without discarding nearby geometry. Eight quantized camera headings
reorder only unread tasks and the display budget; already cached Region data,
in-progress reads, edit history and source NBT are not changed. Priority never
acts as a visibility or source-data filter.

## Minecraft instance resource auto-detection

With 3D spectator enabled, MCEdit2 probes the exact version-matched vanilla
Java client JAR, first near the world save and then at known Minecraft,
PrismLauncher and MultiMC paths. When the world is inside `saves/`, its parent
game directory is also used for a **bounded, flat** read-only scan of
`mods/*.jar` files that actually contain `assets/<namespace>/...`;
disabled JARs and plain Java libraries without assets are skipped.
`kubejs/assets` is treated as a generated resource layer. Only `file/...`
entries enabled in `options.txt:resourcePacks` are loaded from `resourcepacks/`,
in their recorded low-to-high order. No Mod code/scripts are executed and no
network/download or disk-wide discovery occurs. The UI can rescan the same
instance without clearing the previous completed atlas. Switching game
instances invalidates the old atlas and discards stale worker results.

**Core renderer scope:** Mod assets use the same declarative vanilla Java
blockstate variants, weighted choices, multipart conditions, JSON parent
inheritance, elements, per-face UV, rotation, tint and `textures` aliases.
Only standard PNG and `.mcmeta` sources are imported by the production
resource loader. If a foreign-namespace block has no resolvable Java model,
has an unresolved parent, specifies a custom `loader`, or has invalid
geometry, the viewer uses the fixed purple/black missing-texture marker on
a conservative placeholder rather than guessing a same-named PNG. Missing
individual JSON faces remain absent, not invented. Namespace preservation
prevents accidental cross-Mod sprite borrowing.

Custom Mod geometry, OBJ/MTL, glTF/GLB, block-entity renderers, dynamic
NBT-dependent meshes and executable client render hooks are **not enabled**
in the production asset path. They are reserved for a later opt-in plugin
rendering interface; no current plugin support is implied.

## Exact 3D chunk seam visibility

The native 3D spectator streamer now uses the original world reader's
block-face occlusion predicate when independently loaded exact chunks
meet. Identical glass and leaves hide only their proven internal faces;
ice/stone and glass/stone keep the original visible opaque face, while
full slabs, partial slabs, eight-layer snow, fluids, and unfamiliar mod
blocks follow the same conservative rules as within a source chunk.
Read-only distant LOD samples never count as an exact occluding neighbor.
This avoids both visible internal seams and accidentally missing faces
when a 3D camera crosses Region/chunk boundaries. The scanner borrows
source blockstate data instead of duplicating all palette strings.

## Corrupt resource-pack fallbacks

Directory packs, ZIP packs and client JARs now share the same resource
classifier, per-entry byte limits, and texture/model ingestion path. An
invalid modern `textures/block/` PNG cannot reserve an ID or suppress a
working legacy `textures/blocks/` PNG in the same pack. Valid modern
textures still take precedence independently of ZIP member order. If a
higher-priority pack has an unreadable block image, the previously loaded
lower-priority image and its animation metadata remain intact. This is
display-only: assets are not extracted or written back. Four regression
tests cover corruption, archive ordering, layered fallback, and identical
directory/ZIP lookup behavior.

## Native shape fallback for connected vanilla blocks

When a texture-only resource pack omits the original Java blockstate/model
JSON, spectator rendering uses state-aware, read-only geometric fallbacks
instead of full placeholder cubes for glass panes, stained glass panes, iron
bars, low/tall walls, fence gates and wooden fences. North/east/south/west
connections, wall post flags, gate facing/open/in-wall flags and independent
upper/lower fence rails all affect the native 3D vertices. This is a bounded
fallback, not a substitute for arbitrary modded geometry or the complete
Minecraft special renderer. When genuine resource-pack JSON resolves, its
declared geometry **always wins**. Unknown namespaces stay conservative.

Seven Rust regressions exercise the original connection-state geometry,
resource-model precedence and the actual perspective mesh's open/closed
passage. No source NBT is changed, and the distant sampled LOD is untouched.

## Original stair corner geometry

For vanilla stair blocks without a resolvable Java blockstate/model JSON,
the native 3D spectator renderer now respects the original `shape` property:
`straight`, `inner_left`, `inner_right`, `outer_left` and `outer_right`.
Combined with four `facing` directions and `top`/`bottom` halves, these
produce 40 tested source-state arrangements rather than one rectangular
tread. The upper or lower step is split into exact quarter-block cuboids
to preserve inner corners and prevent oversized outer corners; exact shared
inner faces remain subject to mesh occlusion. Unknown shape/facing values
fail closed to a conservative selection cube, and custom Mod namespaces
are never assigned vanilla stair geometry. Loaded resource-pack JSON model
elements remain authoritative instead of using these fallback boxes.
Tests include a world-space spectator mesh inspection of all five shapes.

## Native Java double-height door shape fallback

When the installed Java client or a texture-only resource pack does not
provide a resolvable original blockstate/model JSON, native spectator mode
renders vanilla wooden, metal and copper double-height doors as a thin
3/16-block panel rather than a full opaque cube. The source `facing`, `open`,
`hinge` and `half` properties control edge placement for both door blocks.
Invalid or foreign-namespace states conservatively retain the previous
fallback; existing blockstate/model JSON always takes precedence. Source
world data, block entities and other editor state are never modified.
Regression tests cover all 4 directions, both hinges, open/closed panels,
upper/lower alignment and the real world-space GPU vertex extents.

## General Java block JSON 3D model coverage

The native 3D model/texture pipeline no longer assumes that every block
model or sprite lives under `models/block/` or `textures/block/`.
Blockstate files can reference **namespaced model parents and elements** in
`models/item/`, `models/custom/` or other original JSON model families.
Their face textures can resolve namespaced sprites in `textures/item/`,
`textures/entity/` or other Java resource folders. Canonical
`textures/block/` and historical `textures/blocks/` names retain their
existing shared block sprite IDs without colliding with non-block families.

Authentic model elements may extend outside the source block, within Java's
bounded [-16,32] source model unit range. They retain their exact
read-only 3D vertex coordinates and declared UVs; models with up to 256
elements each and 512 elements total per blockstate are supported before
conservatively falling back rather than consuming unbounded CPU/GPU memory.
Model parent recursion, resource path validation, archive byte budgets and
render face limits remain enforced. New regressions check directory/ZIP
model/PNG parity, nested item/custom model inheritance, namespaced item and
entity textures, large multipart models, malicious paths, and the emitted
GPU spectator vertices for a real out-of-cell Mod block model.

This improves arbitrary static JSON-defined block display but **does not
claim every special renderer is implemented**. Minecraft block-entity
renderers (e.g. chests, banners, signs and beds with dynamic NBT content),
entity model rigs, shader-specific animated geometry, runtime OBJ/custom
loaders, multipart non-JSON renderer hooks and true tiled UVs outside
0–16 still require specialized rendering work. No remote shader, executable
or Mod code is loaded. Render samples never become edit sources.

## Generic static atlas-safe repeated UV rendering

The perspective spectator renderer can now display original static JSON face
UV ranges in [-256, 256] without incorrectly stretching or clamping a texture
at the 0–16 boundary. A renderer-agnostic partitioner splits each face along
the original 16-unit texture periods, preserves UV quarter turns/reflections
and exact 3D vertex geometry, and maps each subface back into the original
atlas slot. Because GPU atlas sampling remains clamped, UV repetition never
samples a neighboring Mod sprite. Work is capped at 256 tiles/source face
plus the existing total mesh face limits. Independent tests and a resource-pack
to GPU-vertex integration test validate actual repeated quads and UV bounds.

This is generic static model handling, not per-block renderer special cases.
Dynamic NBT-dependent block entity shapes and non-JSON loaders still need a
source-aware plugin/model contract; no Mod executable render code is loaded.

## Legacy experimental special-model parsers (not enabled by the app)

The following historical sections describe **test-only experimental
implementations** retained in source. The desktop application's production
automatic and manual resource entrypoints now use `open_stack_java`,
which does not ingest OBJ, MTL, glTF, GLB, sidecar model images or binary
buffers. Those experimental parsers do not participate in normal Mod
rendering; a future plugin interface would need to opt in explicitly.
Historical tests using the legacy `open_stack` path do not indicate
production renderer support.

The universal resource-pack model path now includes a **guarded static OBJ
backend** selected by `forge:obj` / `neoforge:obj` in the original block
JSON. Source OBJ files under `assets/<namespace>/models/` are parsed
read-only; their declared vertex/UV data, material names and blockstate
X/Y rotations are mapped to the same texture atlas and wgpu spectator scene
as ordinary Java cuboid models. No resource file is executed or extracted
and no external `mtllib` or remote references are followed. Resource bytes,
vertex/triangle counts, coordinate ranges and available material names
remain bounded. Unknown custom model loader identifiers are not guessed.
Existing GPU batch sorting expects six vertices per face; each static OBJ
triangle uses one degenerate trailing triangle for coherent transparency
grouping. Integration tests exercise real resource JSON -> OBJ -> atlas
material -> 3D GPU vertex flow, without hardcoded block identifiers.

This backend is **not** a complete Wavefront implementation: this first
static subset supports indexed triangular and quad faces with UVs; complex
polygons, nested MTL pipelines, vertex normals, runtime loaders, bones and
dynamic block entity renderers still need independent, source-verified
render backends. Current OBJ UV values outside the sprite's [0,1] domain
are clamped; the general repeated-UV path presently applies to Java JSON
faces, not arbitrary OBJ triangles.

## Static resource-renderer adapters (non-executable)

This branch now has three generic static mesh data paths: existing JSON
model elements, source-declared OBJ + same-pack MTL materials, and embedded
glTF 2.0 / GLB mesh primitives. glTF buffers must be bundled into the GLB or
base64 data URI; remote and relative external buffers are not fetched.
Supported meshes use finite POSITION / TEXCOORD_0 coordinates and bounded
triangle budgets. OBJ/MTL handles static diffuse RGB, opacity (d/Tr),
material names and same-pack same-stem map_Kd mapping; a map_d alpha-map
identifier is validated but per-pixel map_d compositing is not implemented.

All OBJ/glTF source triangles pass through the same bounded atlas-safe
integer-period UV clipping: crossing tile boundaries creates interpolated
triangles that remain inside their material's own GPU atlas sprite slot,
without enabling global sampler wrapping. MTL opacity is sent as a Blend
material in the existing GPU transparency pipeline.

Any custom Mod renderer can opt into a **declarative** static fallback via
its JSON block model, without a built-in per-block registry. For example:

```json
{
  "loader": "othermod:custom_client_renderer",
  "mcedit2:static_renderer": {
    "format": "glb",
    "model": "othermod:models/custom/structure.glb",
    "textures": {"particle": "othermod:item/structure"}
  }
}
```

The adapter is resolved only from supplied JSON and exact resource asset
identities. It does not execute Mod code, infer opaque NBT-driven rendering,
or promise compatibility with arbitrary custom client-only algorithms.
Remaining gaps include full glTF scene node hierarchy, external buffer
resolution, PBR/normal/metallic material shading, independent MTL map_d
opacity images, arbitrary non-triangulated polygons and dynamic geometry.
This pass deliberately does not add animation.

## glTF 2.0 static scene hierarchy and pack-local buffers

The static glTF/GLB loader traverses the **selected scene's actual root
nodes**, composes parent/child TRS or column-major matrix transforms, and
renders only reachable mesh instances. It rejects cycles, excessively deep
trees, invalid node references and nonfinite/out-of-range geometry. glTF
bufferViews may reference up to 32 declared buffers. Embedded GLB BIN and
base64 buffers remain supported; external `.bin` references resolve strictly
within the same loaded resource-pack model directory, from directory, ZIP,
or JAR content. Resource member order is immaterial: source binaries are
read-only cached and glTF meshes are assembled only after all layers load.
Paths with traversal, absolute roots, backslashes or URI schemes are refused;
no network or arbitrary filesystem reads are performed. Combined buffer
memory is bounded and unresolved model references produce warnings, not
world-save changes. Scene hierarchy, buffer bounds, traversal, and unordered
ZIP references have Rust regression tests. Skinning and animation are
intentionally excluded from this static renderer.

## OBJ/MTL opacity atlas and static material parameters

OBJ Wavefront `map_d` is now applied as a **real per-pixel opacity map**
instead of only being recorded as a parsed filename. After all resource
layers and any source animation frames have been resolved, valid
`map_Kd`/ `map_d` pairs are combined into immutable RGBA sprites before
GPU atlas packing: grayscale mask luminance or explicit PNG Alpha scales
the original diffuse Alpha channel. Different texture dimensions are sampled
in normalized local texture coordinates. Synthesized sprites share the
ordinary transparent/cutout material path and total atlas memory budget.
Unresolved masks produce warnings and leave the original material intact.
No source PNG, save NBT or Mod executable is changed.

The OBJ MTL metadata reader now validates diffuse, ambient, specular and
emissive colors (`Kd/Ka/Ks/Ke`), `d/Tr`, `Ns`, `Ni`, `illum`, and
the common `Pm/Pr` metallic/roughness extensions. glTF/GLB material
metadata reads `baseColorFactor`, alpha mode/cutoff, double-sided,
metallic/roughness factors, and emissive factors. Base-color RGBA affects
the existing static GPU vertex tint. **True PBR illumination is not
complete**: image-based lighting, tangent-space normal maps, metallic-
roughness texture sampling, and correct per-material alpha cutoff are
not yet implemented in wgpu. Reading these metadata fields is not
equivalent to full PBR shading. Animations and Mod client renderer hooks
remain deliberately out of scope.

## Minecraft-like fullbright spectator material rendering

The perspective 3D spectator shader treats original block/model textures
and declared static base-color factors as **fully illuminated**. It does not
darken nearby or distant surfaces based on world sky/block light, time of
day, camera direction, metallic roughness, or simulated ambient occlusion;
the GPU depth buffer and transparent face ordering still operate normally.
The goal is vanilla-style recognizable block/resource textures with flight
navigation, not an independent PBR lighting engine.

The common GPU material-mode flags now preserve standard glTF
`OPAQUE` (ignore sampled Alpha), `MASK` (per-material `alphaCutoff`),
`BLEND` (sort and alpha-compose) and `doubleSided` (selective backface
discard) without changing the geometry or atlas layout of existing
Minecraft/OBJ faces. Ordinary cutout geometry retains the 0.5 cutoff.
Material alpha metadata comes from resource-provided glTF/GLB JSON, not
block ID guesses. Dedicated Rust tests check encoded modes, cutoff values
and correct GPU sort classification with embedded material flags.

This is intentionally **fullbright**, so glTF metallic/roughness,
normal maps, BRDF reflections and physical light transport do not alter
the voxel display. These properties remain readable source metadata, not
a claim of full PBR appearance. No Mod-specific renderer is added here.

## Automatic glTF base-color sprite binding and source-verified LOD reuse

Static glTF 2.0 / GLB materials with ordinary
`pbrMetallicRoughness.baseColorTexture.index` now resolve their source
`textures[index].source -> images[source].uri` references without Java-side
manual `textures` entries. Unnamed materials get stable index-based source
identities instead of all collapsing to `particle`. Image URIs resolve
relative to their originating glTF path **inside the same resource-pack
namespace**, with bounded canonical `..` handling and strict rejection of
network/absolute/escaping paths. Textures in `assets/<ns>/models/**.png`
receive noncolliding `<ns>:__gltf/...` atlas keys; references into the
ordinary `textures/` tree reuse existing Java texture IDs. Explicit
resource-model material mappings remain higher priority. This first
automatic image path handles pack-local external PNG URIs; embedded
`image.bufferView`, base64 image data URIs, alternate texture-coordinate
sets and glTF sampler filters are **not yet** bound automatically.

For camera-driven terrain streaming, a cached higher-fidelity original
source chunk now supersedes its coarser sampled visual patch whenever the
complete exact representation fits the current mesh budget. Conversely,
while an exact request is still reading, a cached lower-fidelity source
patch remains visible as a temporary bridge. Each source chunk contributes
only one chosen patch to a rendered frame, and patches overlapping the
exact near-source viewport remain excluded. This does not synthesize
missing source chunks, add any editable LOD data or imply full exact
64-chunk-radius rendering. The far horizon remains a sampled,
non-editable visualization; source chunk completeness is recorded.

Regression coverage includes safe glTF image URI traversal, material
name collision avoidance, pack-to-atlas glTF image auto-binding with
two distinct static scene nodes and spectator vertices, and source
chunk LOD transition preferences/voxel budgets. The latest GitHub
workspace still requires dedicated full-head Cargo and real-save GPU
visual testing before declaring rendering equivalence.

## Fullbright spectator flicker and missing-texture diagnostics

The compact 3D sidebar now distinguishes the **Textures on** preference
from a successful resource-pack load. If no original client JAR or resource
pack is installed/recognized, it explicitly reports that the current view
is a palette-only preview and offers the existing **Open ZIP/JAR** or folder
controls. When assets are loaded, it reports the PNG, atlas-page and model
counts and shows any asset discovery/decode error in the compact sidebar.
Original block textures cannot be inferred from saved Java block IDs alone.

The streaming GPU viewport preserves the **last complete spectator mesh and
its matching source coordinate origin** until a newly streamed source scene
has finished meshing. Source updates are coalesced while the single GPU mesh
worker is active; this prevents every incoming chunk batch from cancelling
the only work in progress and leaving a blank frame. The newest ready mesh
takes priority automatically, and original world/texture-source invalidation
discards any stale fallback so it cannot display wrong atlas pages or worlds.
Camera movement continues to update the camera uniform rather than
rebuilding complete meshes. The handoff regression checks both the old
origin while waiting and new origin when the replacement arrives.

This fixes empty-frame mesh handoff artifacts; it does not claim full
Minecraft client rendering fidelity from distant sampled LOD, nor does it
invent PNG textures when none have been supplied. Full 3D visual validation
against a real world, client resource JAR, and GPU remains a separate task.

## Advanced tools: bilingual grouped interface and native file pickers

The advanced editor is now organized into six selectable tool areas rather
than showing every operation in one continuously stacked form: world/block
inspection, GPU/texture/3D scene controls, block entity NBT/entities,
2D/vertical maps, staged block editing, and file exchange/Region recovery.
The active area is explicit and the navigation, buttons, descriptive
instructions, file-path placeholders and safety notices support both
Chinese and English with the existing language toggle. Source world and
dimension selection remains visible at the top.

All ten advanced import/export file-path families now have a native
**Browse** action; the eight file families that export data also offer
**Save as**. Covered paths include BE batches/journals/snapshots, entity
snapshots, clipboard, MCEdit/Sponge schematics, Region plans, plugin
manifests and edit journals. Existing-world and resource-pack pickers are
also exposed in the advanced interface. Dialogs use `rfd` on separate
threads, with conservative filename-extension filters. Cancelling a
dialog never changes the current value, and manual path typing is still
supported. File selection alone never triggers world modification;
the original explicit preview/stage/save confirmations remain mandatory.

Rust offline regressions exercise Chinese/English tab states and bounded
file-picker extensions. UI navigation does not reset selection, staging,
undo/redo, loaded worlds or source NBT. This changes UI and path entry, not
the behavior or safety requirements of any editor operation.
