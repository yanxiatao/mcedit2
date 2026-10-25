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
or client JAR paths separated by `;`, **lowest priority first**. If the active
Java world has a locally discoverable vanilla client JAR, it becomes the lowest
layer automatically. Upper packs override only matching block PNG, model or
blockstate JSON identities; missing assets still use the original lower layer.
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
- Load PNG textures and Java blockstate/model metadata from local resource-pack folders, ZIPs or a known client JAR. Mixed-resolution, multi-page GPU atlas packing preserves per-face atlas page and UV mapping. Unsupported model features may fall back; no shader or entity-render parity is promised.
- Use in-memory edit groups, preview/undo, native source-validation and deliberately guarded save paths. World writes must not occur implicitly from inspection or a resource-pack load.

Read [COMPATIBILITY.md](COMPATIBILITY.md) for the detailed data-version/storage safety policy. See [original-format regression fixtures](../test_files/README.md) and `crates/mcedit2-world/tests/` for native regression coverage.

## Migration status

This is an active rewrite, not complete MCEdit2 feature parity. Old Python sources, Sphinx docs, PySide/Cython setup and Python-only benchmarks/tests are intentionally absent from **this branch**. They remain available in the original branch/history. Do not use the old Python app instructions for building the Rust binary.

The latest code must be checked with the toolchain before a distributable build is described as verified. The previous Windows development ZIP does not automatically contain newer source changes.

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

## Installed client auto-detection

With the 3D spectator view enabled, MCEdit2 attempts to load the exact
version-matched vanilla Java client JAR, first from paths adjacent to the
world save and then from conventional Minecraft, PrismLauncher and MultiMC
install locations under the user's OS application data or home directory.
Only known candidate filenames are probed; the editor does not scan disks,
search the network or modify the installed launcher/client. If no matching
client is installed, a diagnostic tells the user to select a pack/JAR
manually. A manually selected stack still overrides its vanilla base.

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

## Static special-model loader, independent of block identity

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
