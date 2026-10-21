# MCEdit2 Next — experimental changelog

This changelog covers the *Python 3 Next* application on the
`python3-modern-editor` branch. The original legacy MCEdit2 source remains
available and is not automatically replaced.

## 0.2.0a0 — 2026-10-20

### New GUI and visual editing
- Experimental OpenGL 3.3 voxel viewport, with face culling and bounded
  visible area; switch between the original Next 2D slice and the 3D view.
- Orbit (right drag), zoom (scroll), WASD/arrow pan, 3D ray picking and
  drag-to-select with X/Z/Y boundaries.
- Left-click replace, Ctrl+left-click place against the clicked face.
- Three-dimensional selection display, continuous painting as one undo step,
  and plugin brush-mode integration.
- Read Minecraft JAR/resource ZIP/mod JAR assets; render basic full-cube
  textures from blockstate variants and model texture references. Unknown,
  multipart, rotated and noncube models still use color-cube fallback.
- Choose render radius, load layered resource assets, and change vanilla or
  discovered namespaced dimensions. Initial view focuses on world spawn.

### Native data and safety
- Native Anvil/NBT backend, not Amulet; preserve unedited BE and modded NBT
  fields byte-for-byte while editing supported Java 1.13+ paletted sections.
- Block-state palette support up to 4096 IDs/section, negative Y,
  compression formats 1–3 and local region backups.
- Detect external region edits and changed session locks; attempt OS-level
  lock of `session.lock` during save to reject active Minecraft sessions.
- All unknown/unsupported modern chunk formats fail closed rather than
  silently dropping NBT. Manual backup and offline world copies remain
  mandatory.

### Plugin system
- Opt-in Python 3 `plugin.toml` packages with load/unload/reload, command
  options dialogs and compatibility syntax for MCEdit2 SimpleCommandPlugin.
- Plugin commands and brush strokes commit through the editor undo
  transaction layer, rather than arbitrary direct block writes.
- Registration APIs for tools, generators, widgets, inspectors and custom BE
  references remain, but their full Qt6 GUI/editor behavior is **not ported**.
- Read-only Block Entity NBT inspector, including raw NBT access for plugins.

### Validation
- Python 3.12/3.13/3.14 unit tests, synthetic modded BE round-trip tests,
  cross-process session.lock checks, OpenGL software rendering + frame capture,
  Qt6 PNG texture atlas tests, and Windows portable ZIP CI.
- **All tests are development checks, not a guarantee of safe writes on real
  worlds.** Validate every supported version and modded format with
  actual Minecraft before production use.

### Still required for complete MCEdit2 parity
- Full 3D model/blockstate rotations, transparent sorting, connected and
  emissive textures, resource-pack priorities matching Minecraft, and entities.
- NBT/BE/entity writing with strict no-loss semantics and transactional undo.
- Relight/heightmap changes, new section creation, biome editing, and true
  atomic multi-region updates with recovery.
- Historic pre-1.13 Java numeric-ID and Bedrock LevelDB backends.
- Schematic, structure, inspector and old Qt4 plugin tool parity.
- Real maps, new snapshots, modern Forge/Fabric/NeoForge modded fixtures,
  multiplayer concurrency and desktop usability validation.

**Experimental release: use test copies only.**

## Development follow-up: safer structure interoperability

- Block-only clipboard copying now refuses selected BE locations.
- Added vanilla Java `.nbt` structure import/export with compact palette,
  negative-Y positions, safety limits, version confirmation and undo history.
- BE/entity records are rejected until lossless transfer is implemented.

- Added loss-averse Sponge v2 `.schem` import/export for complete block-only
  structures, including VarInt palettes above 127 entries. Rejects biome,
  entity and block entity payloads rather than silently ignoring them.

## 2026-10-20 — large-world streaming development

- Index generated chunks across all Anvil region files, including
  negative coordinates, without loading their full block arrays.
- Stream decoded Sections on background workers with configurable LRU
  cache size and explicit, bounded whole-generated-world preload.
- Draw Section meshes from persistent GPU buffers, reuse atlas textures,
  avoid rebuilding geometry on ordinary camera movement, and cull
  neighboring Section faces.
- Read only requested chunk sectors instead of entire .mca files.
- Camera navigation can use a view radius measured in chunks rather than
  the previous tiny block cube, plus a far clipping distance of 4096 blocks.
- New Ctrl+G world coordinate jump, English/Simplified Chinese controls,
  and magenta-black fallback tiles for unknown modded textures.
- Restore guarded legacy Java preflattening Anvil numeric-ID Section
  support; read all IDs, only write explicitly unambiguous mappings.
- Some world-size/radius combinations still need LOD and coarse greedy
  meshing; no claim of full Minecraft render-distance equivalence.

## Editing and render optimization batch

- Greedy per-Section face merging plus atlas shader texture repetition,
  dramatically reducing geometry for contiguous cube terrain.
- Move greedy mesh computation to bounded background workers, keeping
  Qt texture/GL operations on the main thread.
- Recreate GPU meshes when returning to decoded, CPU-cached Sections
  after a long camera move; ignore missing Sections as air.
- Add hollow selection shells, sphere and 3D line drawing with undo
  and protected BE rollback, including plugin sphere staging support.
- Expand headless tests and Mesa software-OpenGL smoke checks.

## Experimental BE transfer work

- Explicit BE-aware selection clipboard and paste preserve the original
  arbitrary BE NBT payload, rewriting only root x/y/z integer fields.
- Transactional block/BE staging participates in undo and native Anvil
  region backups. Normal block-only editing still refuses BE locations.
- Strict rejection of duplicate/malformed BE positions, unsupported old
  format, and pre-existing destination BEs. Mod-specific linked references
  and unique machine UUIDs are not rewritten; modded gameplay validation
  is still required before treating copied controllers as functional.

## 0.3.0a1 — Legacy Alpha schematics and migration audit

- Inventory of migrated, partial and unported MCEdit2 features in MIGRATION_STATUS.md.
- Guarded old Alpha .schematic import/export with numeric ID and metadata mapping.
- Refuse unknown mod IDs, ambiguous metadata, entities, block entities, tile ticks,
  nonzero placement offsets and unknown NBT fields rather than silently discard them.
- Add .schematic regression cases and batch CI for format safety.

## Same-version editing correction

- Added native numeric ID + metadata editing for old Java Anvil, including
  unknown IDs and `Add` insertion when a section requires a high ID.
- `.schematic` imported into old numeric worlds retains raw ID/metadata,
  bypassing modern block mapping. Cross-format import stays optional.
- Add save guard for externally modified `level.dat`; regression tests verify
  original world and chunk `DataVersion` bytes remain unchanged.

## Old Java MCRegion (Beta 1.3–release 1.1) migration

- Added native `.mcr` X-Z-Y flat 16×128×16 block arrays, metadata edits,
  original region save/backup and full-map indexing without `.mca` conversion.
- Added original-format read/save/reopen and untouched BE/light/custom NBT
  tests, and refusal of ambiguous converted worlds.
- Added legacy numeric ID+meta controls, MCRegion Y limits, and 2D
  right-click block sampling without modifying blocks.

## 0.3.0a2 — 2026-10-20

- Implement old Java MCRegion `.mcr` original-format block edits (16×128×16
  XZY layout), save, backup, reopening, and whole-world chunk indexing.
- Add explicit legacy numeric block ID / metadata input controls and a
  right-click block sampler in 2D (Ctrl+Shift+B at view center).
- Restrict old `.mcr` heights and native IDs; detect ambiguous converted
  worlds with both old and new region formats and fail closed.
- Preserve original `level.dat`, BE, entities, light, heightmap, and unknown
  NBT byte-for-byte. Still requires game-side real-world validation.

## 0.3.0a3 — Selection editing parity (2026-10-20)

- Added native block-only Clone Selection and Move Selection with X/Y/Z
  offsets, overlap-safe source snapshots, and single-transaction undo/redo.
- Added six-connected selection-bounded flood fill to the Qt6 Edit menu.
- BE positions in source/destination are rejected, even if state matches.
  A native no-mutation preflight checks section availability, coordinate
  height and ID/metadata compatibility before multi-position staging.
- Selection follows a successful Move; source and destination identities
  are preserved without DataFixer/format conversion.
- Added synthetic and native save/reopen regression tests for modern Java
  Anvil and old MCRegion, including opaque modded NBT preservation.

## 0.3.0a4 — Original-version block rotations and mirrors

- Added guarded 90° Y-axis clockwise/counterclockwise selected-block
  rotations and X/Z mirrors with 3D selection frame updates.
- Transform native modern vanilla facing/axis/rail/stair/door/connection
  properties and a verified subset of legacy stairs/log/torch metadata.
- Reject unknown mod block orientation, unsafe old numeric IDs, BE source or
  destination coordinates, invalid Sections and oversized transformations
  before changing any block.
- Added native Anvil and old MCRegion save/reopen tests, original NBT
  preservation, rotational inverse and geometry/undo regression coverage.

## 0.3.0a5 — Restricted native entity placement and Windows freeze

- Disabled Windows Actions builds completely until MCEdit2 full migration;
  removed the PR label and blocked even manual dispatch jobs.
- Added byte-precise same-chunk simple vanilla entity Pos editing via
  older chunk-contained and newer separate entity region NBT.
- Preserves UUID and unknown data, refuses attached/mounted/modded entities,
  and rejects cross-chunk relocation before staging.
- Uses existing undo/redo and original-region save/backup; added synthetic
  modern/legacy reopen, inverse undo and untouched NBT checks.
- UI offers an explicit position editor from the read-only entity NBT list.

## 0.3.0a6 — BE NBT scalar editing

- Native BE NBT tree supports explicit type-preserving Byte/Short/Int/Long/
  Float/Double/String edits; no whole-NBT reserialization or DataFixer.
- Expanded native writer to permit in-place existing BE field updates in
  numeric Anvil and MCRegion, not only modern paletted Anvil.
- Prevent editing identity, position and relationship fields; reject
  malformed values, missing tags, type changes, non-scalar edits.
- Same undo/redo, backup and save/reopen, with unknown modded NBT/sections
  preserved bit-for-bit; synthetic per-format tests added.
- Continue WITHOUT any Windows build. Linux CI only until full migration.

## 0.3.0a7 — Packed-array editing

- Per-index edits for Byte_Array / Int_Array / Long_Array, preserving
  original type, length and all unrelated raw bytes.
- Added Qt6 bounded lazy array-index picker, without large array expansion.
- Native undo/save/reopen regression tests for modern Anvil and MCRegion.
- Windows builds stay disabled until full feature migration.

## 0.3.0a8 — NBT structural editing

- Added byte-precise insertion/deletion of scalar Compound fields and members
  of existing scalar-type TAG_Lists, without rewriting unrelated modded data.
- Added typed insertion/deletion of existing Byte/Int/Long Array members,
  modifying only a four-byte count plus the inserted/removed value.
- Added opt-in Qt6 tree dialogs and destructive-action confirmations.
- Changes integrate with BE original payload history (undo/redo) and native
  .mca/.mcr save with backups. Identity/reference tags remain protected.
- Added modern/MCRegion save/reopen and roundtrip loss-prevention tests.
- Full application is still unported; no Windows builds until completion.

## 0.3.0a9 — Restore native backups

- Added guarded original-format `.mca` / `.mcr` region snapshot restoration.
- Generates an inverse safety backup before replacing files, supports undo
  of a restore by restoring that inverse snapshot.
- Refuses pending edits, untrusted snapshot paths, symlinks, external world
  metadata changes and absent region targets.
- Refreshes GUI caches and clears old edit history after restoration.
- Added modern Anvil and MCRegion native restore roundtrip tests.
- Crash-atomic multi-region journal remains a separate future task.
- Still no Windows builds before complete MCEdit2 migration.

## 0.3.0a10 — Native Python 3 tool/generator callbacks

- Added Qt6 Plugin menu entries for callable Python 3 tool/generator
  extensions, each using bounded undoable EditTransactions.
- Ported a built-in checkerboard selection generator to the modern plugin
  callback API; original Java world versions remain unchanged.
- Explicitly refuse running old PySide1 tool/generator class registrations
  until their UIs and Schematic APIs are ported.
- Regression tests cover undo, failure rollback, selection budget,
  missing selection, unloading and legacy class rejection.
- Windows preview build remains disabled until full migration.

## 0.3.0a11 — Native inspector and brush shape extensions

- Enabled Qt6 native block inspector plugin menu and read-only snapshot
  results, including unchanged modded Block Entity NBT payloads.
- Enabled Python 3 brush shape providers using undoable mouse-stroke
  transactions. Added bundled five-block cross example.
- Refused invocation of legacy PySide1 inspector/shape class providers.
- Added rollback, unload, opaque NBT and Qt6 menu smoke tests.
- No Windows preview builds; original-world formats unchanged.

## 0.3.0a12 — Qt6-native plugin widget panel callbacks

- Enabled `widget` Python 3 callbacks from Plugins menu, verifying QWidget
  type and displaying in a closeable Qt6 dialog.
- Added a built-in Qt6 panel provider and GUI smoke coverage.
- Explicitly never instantiate legacy PySide1 widget class registrations.
- No world version conversion or Windows packaging.

## 0.3.0a13 — Opt-in native BE decoder plugins

- Allow trusted Python 3 plugins to register an exact-namespaced NBT Block
  Entity ID decoder; expose enabled decoders in the Qt6 Plugins menu.
- Validate BE root identity before calling a decoder, pass original native
  NBT bytes without transformation, and enforce bounded read-only output.
- Retain legacy TileEntity reference registration without executing old
  PySide1 classes. Add an opt-in vanilla chest decoder example and tests for
  modded unknown binary fields, ID mismatch and plugin unload.
- No Windows build or production world compatibility claim.

## 0.3.0a14 — Bounded native block search

- Search within a selected X/Y/Z volume without generating chunks or writing
  BE/metadata, with a strict per-operation volume budget.
- Match native block names across modern blockstate properties, or opt into
  exact complete states / legacy numeric metadata.
- Show bounded coordinate results (up to 4096) and jump to selected match
  in 2D/3D. Count all matches even when only a subset are displayed.
- Add tests for negative Y, property variants, original numeric metadata,
  non-mutation, and oversized search refusal.
- Windows builds remain disabled; real-game validation is still required.

## 0.3.0a33 — Render-only water/lava surface geometry

- Retain original Java blockstate `level` values but visualize surface
  height and sloped corner edges in the threaded 3D Section renderer.
- Omit contiguous fluid interior faces, clip exposed sides, account for
  cardinal-neighbor Sections and refuse diagonal sample fabrication.
- Resolve explicit vanilla still/flow texture names with reusable atlas
  indices, falling back to colored faces for missing resources.
- Crop vertically stacked fluid PNGs to the static first animation frame;
  full animation, biome tint, exact liquid corner tessellation and
  face-level transparency sorting remain incomplete.
- Add source-only fluid mesh and Qt texture regression specifications.
  No full tests/CI or live world validation performed in this batch.
- No NBT writes, no Bedrock conversion, no Windows build; Draft PR only.

## 0.3.0a34 — Guarded original world/player NBT editing

- Port selected World Info / Players scalar editing to the Qt6 docks:
  original game mode/flags, spawn/time fields, difficulty, world name and
  safe player health/XP/food statistics, each with an exact original NBT type.
- Preserve all unmodified and unknown modded NBT tags via byte-span edits.
  No DataFixer, original world version conversion or inventory write.
- Use Minecraft session-lock and stale-view CAS guards, reject pending
  region edits, create original complete gzip backups and atomically replace
  a single native metadata file.
- Restore editor-created metadata backups through a dedicated UI with an
  inverse safety backup and same-version validation.
- Source regression tests added; full CI and real-world verification
  deliberately deferred. No Windows build or new PR/branch.

## 0.3.0a35 — Guarded native ItemStack count and slot edits

- Qt6 Players dock now edits existing original ItemStack `Count`/`count`
  and moves entries to **empty existing native slots** in the same
  `Inventory` / `EnderItems` list.
- Original tag types, item ID, modded NBT/components, other stacks, list
  ordering and world `DataVersion` are retained. Changes use original
  byte-span scalar patching, not a reconstructed ItemStack schema.
- Refuse occupied/duplicate/unknown slots, modded custom layouts,
  nonnumeric or ambiguous count tags, unsafe values and stale player NBT.
- Commits use the a34 session lock, full original gzip backup, guarded
  atomic replacement and explicit inverse restoration. Inventory writes
  remain separate from region Undo/Redo.
- Source tests committed but not run as a full CI/game validation batch.
  No Windows artifact and no new PR/branch.

## 0.3.0a36 — Original player inventory swaps and transfers

- Added loss-averse original ItemStack **same-container swap** and
  **Inventory ↔ EnderItems transfer**, including explicit occupied-slot
  exchanges in one original player NBT patch.
- New native byte-span helper preserves all unknown modded item/component
  NBT and other player/item list members while changing only Slot and the
  two affected List memberships.
- Existing original whitelist, item identity, count, slot uniqueness and
  list structure are validated. Unknown modded slot layouts fail closed.
- Migrated corresponding Qt6 Players dock flow and complete gzip
  backup/inverse restore integration. No EditSession region undo promises.
- Added source tests for unknown-item preservation, swaps, empty destination
  lists, unsafe argument refusals and disk-save restoration. Full CI and
  Minecraft-side fixture validation deferred; Windows build disabled.

## 0.3.0a37 — Original ItemStack split/merge/delete

- New `inventory_stacks.py` supports existing original Java player
  `Inventory`/`EnderItems` stack splitting into a free slot, strict
  bit-identical-except-Slot/Count merging, partial/full stack exhaustion,
  and explicit existing ItemStack deletion.
- All unknown modded ItemStack tags, original list member bytes and
  player NBT siblings are preserved unless the user explicitly deletes
  that one entry. Unsupported inventory schemas and unequal component
  bytes fail closed; item-specific maximum stack size is not inferred.
- New Qt6 Players dock split/merge and deletion actions with explicit
  user confirmation, complete gzip inverse backups and stale file/session
  guards. Does not alter region Undo/Redo.
- Source regression specifications added; full test and real-game
  validation matrix and Windows packaging remain deferred.

## 0.3.0a38 — Native Java filled-map editing

- Added `native_map_edit.py`: byte-exact sparse edits of the original
  `data/colors` TAG_Byte_Array and whitelisted original scalar map
  metadata. Unknown mod tags, map geometry and other pixel values
  remain unchanged in the uncompressed native NBT representation.
- Added Qt6 map pixel painter (drag, brush raw palette code, rectangle,
  sample, discard staging) and guarded `scale`, lock/tracking and center
  coordinate editing with existing map NBT viewer.
- Each immediate map_N.dat save is guarded by Minecraft session.lock,
  unchanged level.dat, stale original map data, pending-edit checks, exact
  original compressed gzip backup and atomic replacement; inverse map
  restore has a dedicated browser/button.
- Synthetic source-level tests are committed but **not run**; modern
  palette compatibility and game-side world reload need later validation.
  Windows packaging stays disabled; Draft PR remains unmerged.

## 0.3.0a39 — Entity yaw/pitch and grouped same-chunk move

- Ported byte-preserving original two-Float entity `Rotation`
  edits for the same conservative standalone vanilla subset used
  by the existing Pos editor. Unknown NBT, UUID and Pos bytes
  remain unchanged; missing rotation and linked entities fail closed.
- Original entity index/UI now expose yaw/pitch. Native Anvil and
  EditSession support staged region saves, one-step undo and redo,
  with Pos-only older entities still compatible.
- Qt6 entity list supports multiselection and bounded XYZ offset
  translation, preflighting **all** entities before changing the
  staged world. Each successful group is one Undo/Redo record.
- Added source regressions for exact original NBT retention,
  unsafe target refusal, no partial batch mutations, native
  entity-region backup and save/reopen. Tests not executed in the
  full Qt/Mesa/game-side matrix. Windows builds remain disabled.

## 0.3.0a40 — Original standalone entity NBT scalar controls

- Ported guarded native scalar edits for existing Byte flags `NoAI`,
  `Silent`, `Glowing`, `NoGravity`, `Invulnerable`,
  `CustomNameVisible`, `PersistenceRequired`, and Float
  `Health`/`AbsorptionAmount` without replacing original entity NBT.
- The original Qt6 entity NBT tree gains an explicit whitelist-only
  edit action. Nonexisting, wrongly typed, linked/modded and protected
  fields are refused. Untouched original modded bytes and UUIDs remain
  byte-identical.
- Native Anvil staged edits and actual save verify the same
  whitelist, chunk association and original data, with existing
  region backups and one-edit Undo/Redo records.
- Added focused source regression cases. CI/Mesa/game reload
  and Windows packaging remain deferred.

## 0.3.0a41 — Original Java dropped-item/XP-orb deletion

- Implemented original NBT-List member deletion for native standalone
  dropped items/experience orbs, preserving all other original entity
  compound bytes, modded chunk NBT and DataVersion.
- Multi-select Qt6 delete action with strict type/linkage/original
  chunk checks; no custom mobs, graph migration or UUID rewriting.
- Staged deletions retain *stable pre-delete list indices* for normal
  entity Pos/Rotation/scalar edits on survivors. Whole selection is
  preflighted and reversible through EditSession Undo/Redo **until
  Save**.
- Saving creates the existing full original region backup and clears
  pre-save history involving native List contraction, to prevent
  invalid index-based Undo after Save. Use File → Restore native
  region backup… to recover a committed delete.
- Added source-only preservation/refusal/backup/reopen specifications.
  No full CI, game reload or Windows build performed.

## 0.3.0a42 — Guarded native dropped-item / XP-orb clone

- Original Java dropped-item and experience-orb duplication is now
  available within the **same** original chunk. Original UUID
  IntArray[4] or UUIDMost/UUIDLeast Longs are rewritten with fresh
  UUIDv4 bytes; original item/modded NBT tags are copied losslessly.
  Unknown UUID/linkage schemas, modded/linked entity IDs and
  cross-chunk targets remain refused.
- New Qt6 **Clone selected dropped item / XP orb…** action previews
  the staged new entity; clone entries cannot themselves be edited
  or duplicated until a successful native save.
- Native world and EditSession use UUID-keyed pre-save staging and
  one-step Undo/Redo. The saved region List is appended with only
  validated original Compound payloads and a full original region
  backup; structural history is cleared on successful Save.
- Added source regressions for modern and legacy identities,
  UUID uniqueness, original opaque ItemStack round-trip, native
  region save and backup/reopen; actual full tests deferred.

## 0.3.0a43 — Native original-UUID cross-chunk entity transfer

- Standalone vanilla dropped-item/XP-orb entities may now MOVE between
  **existing Java chunks**, including different region files, with their
  original UUID and unknown/modded ItemStack NBT unchanged.
- Both existing original source and target formats must match. Linked,
  modded and ambiguous identities, missing destination chunks and
  recognized UUID collisions are refused. No target chunks are created.
- Native writer stages one source List removal + destination List append
  in the same preflight/region backup/save transaction. Qt6 adds an XYZ
  target dialog. Undo/Redo works before Save; after Save use the
  original region snapshot restore because List indices changed.
- Added source regressions for original bytes/UUID, negative chunk math,
  missing-target refusal, same-region and cross-region backup/reopen.
  Full version CI/Windows builds and game-world reload remain deferred.

## 0.3.0a44 — Multi-select cross-chunk loose entity transfers

- Added Qt6 **Transfer selected items / XP orbs by offset…**:
  computes destination XYZ for every selected original entity while
  preserving original UUID and opaque ItemStack/mod NBT bytes.
- Bounded native group preflight refuses an entire selection if even
  one target is a missing/nonexistent chunk, same-chunk target,
  duplicate source/index/UUID, unsafe referenced entity or
  conflicting staged change. No partial disk or staged NBT writes.
- Successful batches share one pre-save Undo/Redo record. All touched
  source and target native regions are backed up on Save; cross-file
  replacements remain best-effort rollback, not crash-atomic.
- Added source regression specifications for same-target and
  multi-region batches, exact UUID/NBT fidelity, atomic rejection,
  region recovery and save/reopen. Full tests remain deferred.

## 0.3.0a45 — Repeated selection clone / tiling

- Ported original Clone Tool repeat-count workflow to Qt6 Edit menu
  and dockable Tool Palette with persistent copy-count control.
- All 1–128 copies use **one pre-mutation source snapshot** at
  `n × XYZ step`; overlapping targets use last-stamp-wins and one
  Undo/Redo entry per unique coordinate.
- Block Entity source/target protection, native section/ID preflight
  and existing session block caps reject unsafe edits before staging.
  Other original Java Block Entity and unknown NBT remain untouched.
- Added source regressions for overlap, bounds, Undo/Redo, BE safety,
  original Java Anvil mod blockstates and old MCRegion IDs/metadata,
  backups and reopen. Full CI/game-world testing remains deferred.

## 0.3.0a46 — Rotate / mirror Clone Tool stamps

- Ported **rotate clockwise/counterclockwise or mirror X/Z** of an
  original selection into a clone destination, with optional 1–128
  repeated stamps using one frozen source snapshot.
- Uses the existing strict vanilla/legacy block orientation codec,
  not guessed modded state transformations. Unsupported modded IDs,
  source/target BEs and missing sections are rejected before any
  staging. Later overlapping stamps win with unique Undo history.
- Added Qt6 menu orientation picker and persistent dock selector
  sharing repeat-count controls. Guarded QAction's Boolean signal
  against confusion with optional parameters.
- Added source regression specifications for original Anvil and
  MCRegion data fidelity, rotated rectangle layouts, batch Undo,
  modded/BE failure cases and backup/reopen. Full tests deferred.

## 0.3.0a47 — Native nearest-neighbour Clone Tool scaling

- Added independent **integer XYZ scale factors 1–8** and optional
  1–128 repeated scaled stamps, preserving source and each original
  native BlockState without any modded state normalization.
- Read-only geometry planner supplies scaled dimensions, first/all
  target bounds and full planned-write count; blocks invalid,
  over-limit and out-of-world plans before any file read.
- Source/target BE protection and original native section/ID
  checks prevent partial staging. Overlapping copies use one
  original snapshot and deduplicated single-operation Undo/Redo.
- Added Qt6 menu scaling dialog and reactive bounds preview
  plus persistent tool-dock X/Y/Z scale controls. Tests specify
  original Anvil/MCRegion NBT fidelity and refusal cases.
  Full CI/real Minecraft versions/Windows builds are deferred.

## 0.3.0a48 — Interactive read-only 3D Clone Tool wireframes

- Added pure bounded original-coordinate CloneTool preview plans for
  offset, repeated, rotated/mirrored, and integer XYZ scaled copies,
  with per-copy exact half-open world-space bounds.
- Both basic OpenGL 3D and streamed OpenGL 3D paths now show cyan
  earlier-copy and green last-copy target wireframes, without
  rewriting chunks or forcing streamed chunk remeshing.
- Qt6 Tool Palette offers Preview in 3D / Apply preview / Clear,
  with four modes, XYZ step, orientation, scaling and repeat
  parameters. Camera orbit/zoom remains usable before Apply.
- Preview invalidates on selection/world/dimension/parameter changes;
  Apply verifies the original world and selection and reuses all
  existing native block/BE target preflight and grouped Undo/Redo.
  Invalid native writes remain refused without changing the world.
- Added pure geometry plus offscreen Qt source regression
  specifications. Preview is **a wireframe, not a textured voxel
  or modded BE ghost**. Full Qt/Mesa/real-world tests deferred.

## 0.3.0a49 — Drag-and-snap Clone Tool 3D preview

- Ctrl+Alt+left mouse drags the first clone preview wireframe center
  in world X/Z; Ctrl+Alt+Shift+left mouse drags Y. Preview only,
  never block/BE edits before explicit Apply.
- Added visible white center control and absolute original-world
  1/2/4/8/16-block grid snapping, including negative coordinates.
  Qt6 XYZ controls and both original GL renderers update with the
  planned geometry without remeshing chunks or losing Undo history.
- Repeat and transformed/scaled modes follow existing n×XYZ-step
  semantics. An invalid plan retains the previous preview.
  Apply redoes all native section, BE, ID and orientation preflight.
- New pure-math, camera-ray and offscreen Qt controller source
  regressions added; complete Mesa/game-world tests remain deferred.

## 0.3.0a50 — Drag entire Clone Tool group without changing spacing

- Separate absolute **Group position XYZ** from repeated-copy
  **Step XYZ**. The n-th copy is planned and written at
  original source minimum + group position + n×step.
- Qt6 Tool Palette now exposes both vectors independently.
  Ctrl+Alt / Shift mouse dragging changes only group placement,
  preserving the spacing and footprint of every other clone.
  1/2/4/8/16-block original-world snapping remains supported.
- Native repeated, rotated/mirrored and integer-scaled writes
  accept optional placement (default zero for old callers).
  Apply revalidates original BE/native region safety and groups
  changes as one Undo/Redo history entry.
- Added pure geometry, original Anvil NBT fidelity and offscreen
  Qt controller source regression specifications; no full CI,
  Mesa, game-world or Windows runs executed.

## 0.3.0a51 — Bounded translucent Clone Tool BlockState ghost

- For up to **2,048 planned block cells**, Clone Tool previews now
  include atlas-textured or fallback-colored **semi-transparent
  full-cube voxels**, as well as existing read-only 3D target
  wireframes, mouse drag and grid snapping.
- One original native BlockState snapshot is captured when
  preview begins and reprojected without world rereads on
  3D dragging. Repeat/rotate/mirror/scale and rigid group
  translation follow the same coordinates as actual Apply.
- Both classic and streaming OpenGL renderers have separate
  non-depth-writing blended passes. Streaming uses the existing
  stable texture atlas and cached preview geometry, without
  remeshing chunks. Source BEs, unknown orientation, oversized
  or unreadable selections use wireframe-only fallback.
- **Complex block models and fluids are not yet accurately
  represented** by the per-block preview. Apply still
  independently preflights original BE/section/ID safety and
  creates a normal one-step Undo/Redo edit.
- Added pure projection/texture-geometry and offscreen Qt
  source regressions, without running the full test matrix.
