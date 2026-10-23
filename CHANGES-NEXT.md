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

## 0.3.0a52 — 180° / 270° oriented native clone and selection tools

- Added 180° and clockwise 270° choices to rotated/mirrored
  Clone Tool, dock orientation control and selection transformation
  menu. Existing bounded 3D wireframe and translucent BlockState
  ghost previews use the new mappings without writing world data.
- Expanded strict native horizontal BlockState rotation for vanilla
  stairs, rails, signs, cardinal connections, logs and verified old
  numeric metadata; unknown/modded semantics still fail closed.
- Added source regression specifications for right-angle geometry,
  original region NBT retention, 3D ghost fidelity, protected BE
  failure and original Anvil/MCRegion saved backups and Undo/Redo.
  Full CI, game-world and Windows execution remain deferred.

## 0.3.0a53 — Native Clone Tool integer reduction

- Add XYZ divisor 1–8 nearest-neighbour block-only downscale; dimensions
  are ceil-divided and trailing partial groups are retained.
- Source BE preflight scans **every** original source voxel (even those
  not sampled), while destination BE/Section/ID preflight refuses unsafe
  edits before staging. Native BlockStates are copied without
  conversion and without any entity or BE NBT.
- Qt6 menu/dock divisor controls, read-only geometry preview,
  wireframes and bounded 3D original BlockState ghost share the same
  exact sample-grid semantics. Apply uses independent native checks.
- One Undo/Redo group for overlapping copies. Source regression cases
  include original Anvil/MCRegion NBT, backups, reload, source BE
  refusal, preview parity and size/coordinate guardrails. Full tests
  remain deferred.

## 0.3.0a54 — Clone orientation plus XYZ integer enlargement

- Compose strict 90°/180°/270°/mirror orientation with 1–8x
  nearest-neighbour scaling along the ROTATED output axes;
  support 1–128 repeats and rigid-group placement.
- Preserve immutable original native BlockStates and all unrelated
  chunk NBT; refuse unsupported modded rotation, original BEs,
  protected destinations and invalid/missing native sections before
  staging. Single grouped Undo/Redo and full region backups.
- Add Qt6 geometry dialog, dock preview mode, native Apply routing,
  bounded 3D wireframes and original-texture/color BlockState ghost,
  plus source regressions for original Anvil and MCRegion data.
- Consolidated duplicate independently added oriented-scale
  Python definitions. Full CI/game-version/Windows checks deferred.

## 0.3.0a55 — Guarded vertical Y-mirror clone/selection

- Added `mirror_y` native coordinate/reflection semantics to the
  existing Qt6 in-place selection transforms, oriented copies,
  rotate-then-scale clones and 3D ghost plans. Supported original
  vanilla stairs/slabs/trapdoors swap upper and lower positions,
  verified original logs/invariant blocks retain their native state,
  and original MCRegion stair/slab metadata uses only known flip bits.
- Missing/malformed vanilla properties, modded orientations, doors,
  linked/unknown vertical geometry and unverified old numeric IDs
  are refused. BE source/target preflight and original region backups
  preserve unrelated entity and modded NBT; Undo/Redo remains grouped.
- Added source-only regression cases for vertical involution, modern
  and legacy metadata, read-only ghost fidelity, native save/reopen,
  unsafe refusal and BE protection. Full testing deferred.

## 0.3.0a56 — Ignore source air in native offset/repeat Clone Tool

- Opt-in direct offset and repeated clone mask skips exactly original
  vanilla air/cave_air/void_air, **preserving existing target blocks**
  rather than stamping air. Unknown modded air-like states are not
  guessed or discarded.
- Source Block Entities still refuse the entire operation (even if
  the source state looks like air). BE protection/native target checks
  run on actual write targets; skipped targets remain untouched.
- Overlaps use the unchanged original snapshot and one Undo/Redo
  baseline per unique written block. Qt6 adds explicit menu choices
  and a tool-dock checkbox for these two simple clone modes.
- Added source regression cases for modern Java Anvil/legacy MCRegion,
  exact unknown NBT preservation, refusal/rollback, overlap, and
  original backups. Full test execution deferred.

## 0.3.0a57 — Source-air-transparent rotated/scaled Clone Tool

- The exact vanilla source-air mask now works for rotated/mirrored,
  integer scaled, rotate-then-scale and downscaled native clones,
  not just offset/repeated copies. Unknown/modded air-like blocks
  stay real original states.
- Masked cells produce no target writes; existing terrain and BEs
  beneath the skipped position are preserved. Every original source
  BE is still rejected, including skipped air and unsampled positions.
- Read-only 3D Clone Tool plans, textured voxel ghosts, drag updates
  and Apply reuse one explicit transparency flag. Later overlaps
  cannot erase earlier visible stamp blocks using skipped air.
- Qt6 unified the tool dock mask across every clone mode, added
  per-operation scaled/reduced dialog checkboxes and invalidates old
  previews whenever the option changes.
- Added source regression specifications for all transformed modes,
  original BE/unknown NBT retention, preview fidelity, rejection,
  grouped Undo/Redo and native save/reopen. Full runtime tests remain
  deferred.

## 0.3.0a58 — Destination-air-only Clone Tool writes

- Added an independent **paste into target air only** mask to
  unrotated, repeated, rotated/mirrored, scaled, reduced and
  rotated-scaled native Clone Tool operations; source-air masking
  continues to work alongside it.
- Frozen original target states determine writable cells, even
  when source and destination or repeated stamps overlap. Masked
  occupied target cells and their original BE/unknown NBT are never
  edited. All retained write targets still get BE/section/ID
  preflight, with a single grouped Undo/Redo record.
- Qt6 Tool Palette and clone dialogs expose the option. 3D plans
  retain the flag; Apply revalidates, but masked previews are only
  wireframe outlines, not speculative occupancy-aware voxel ghosts.
- Added native exact-air/BE/overlap/Anvil byte-preservation and
  regression test source. Full CI and real-world builds deferred.

## 0.3.0a59 — Original biome inspector (read-only)

- Added native Java read-only biome slice inspection in the Qt6 File menu.
  Java 1.18+ section biome palettes decode exact original names and quart
  geometry; pre-1.18 byte/int 256-cell and 1024-cell biome arrays are
  reported as numerical legacy IDs instead of guessed biome names.
- Malformed or ambiguous biome storage, unsupported packed indices, duplicate
  sections, missing Y sections and corrupt padding are refused. Unknown
  modded modern biome names remain unchanged.
- Inspecting never stages biome edits, changes block Undo state, writes
  a region or normalizes original modded BE/unknown chunk NBT.
  Synthetic decoder/backend regression source added, not a substitute
  for full Minecraft-version/Qt/Mesa/Windows testing.

## 0.3.0a60 — Original Java 1.18+ biome quart painting

- Add a strict native Java 4×4×4 biome quart writer that changes only the
  selected section's original palette/long-array entries; unknown NBT,
  Blocks/BlockStates, modded BE contents and unrelated metadata stay intact.
- Add Qt6 Edit menu and persistent Tool Palette actions to sample an
  *existing* original biome ID and paint a 3D selection, with explicit
  quart-boundary expansion and disposable-copy warning.
- Preflight at most 128 selected quart cells before staging; preserve
  original chunk/version identity and reject missing target Sections or
  malformed native packing. Batch atomic Undo/Redo, including post-save,
  with existing full-region backups and no added format conversion.
- Add source-only native save/reopen, modded NBT, malformed-data,
  mixed block/biome and cross-chunk rejection specifications. Full runtime
  CI/Minecraft/Qt/Windows validation is still deferred.

## 0.3.0a61 — 2D original/staged biome overlay

- View > Show original/staged Java biomes (2D only) renders stable
  name-colored native Java 1.18+ quart biome cells from original
  Section palettes at the current world Y, with gold borders for
  unsaved staged biome edits. A dialog lists name/color legend
  and error diagnostics; the existing 3D viewport is unchanged.
- Bounded to 64 visible chunks, decoded once per viewport snapshot;
  refreshes after edits, Undo/Redo, Save, dimension/world changes,
  Y or viewport movement. Rendering never creates missing sections
  or modifies block states, BE NBT or region files.
- Malformed, unavailable, or mismatched native chunks are rendered
  distinctly red/crossed rather than assigned guessed biome data;
  unsupported versions explicitly refuse an overlay.
- New native geometry/metadata and offscreen Qt controller
  regression **source** covers caching, staged-state visuals,
  unreadable data and no world writes. Full runtime testing deferred.

## 0.3.0a62 — Exact native Java 3D biome find

- Added conservative exact-ID biome search over selected 3D positions
  at original Java 1.18+ 4×4×4 quart resolution, limited to 32 chunks /
  8192 cells / 256 displayed matches, with exact full hit counts.
  No selection means search the current chunk/current Y only.
- Search handles modded biome names without guessing mappings, optionally
  includes staged unsaved edits, marks missing/corrupt chunks or sections
  explicitly and distinguishes partial scans from complete negative finds.
- Qt6 Edit menu/Tool Palette opens a result dialog with copyable
  diagnostics and navigable matches. Go To switches to the 2D slice
  and selects the exact quart cell; no biome NBT or blocks are written.
- Added native regression and offscreen Qt test **source**; full runtime
  test matrix remains deferred.

## 0.3.0a63 — Lossless marked Java BE structure export/import

- New app-specific marked `.nbt` original Block Entity structure
  transfer alongside the existing fail-closed block-only import/export.
  Direct export embeds original opaque BE Compound bytes under the
  corresponding source block and validates exact byte roundtrip.
- Separate Qt6 File-menu actions explicitly confirm risks; import
  requires exact source/target DataVersion, valid original coordinates,
  preexisting destination chunks/sections and no destination BE.
  Only NBT root x/y/z are adjusted; unknown modded fields are untouched.
  Maximum 512 BE records and 8 MiB of raw payload per transfer.
- Import is one preflighted undoable original-native write transaction,
  saved with existing complete original-region backups. No registry
  remapping, external mod machine link repair, UUID remapping or
  cross-version conversion is claimed.
- Synthetic regressions added as **source** for BE byte preservation,
  disk roundtrip, save/reopen, Undo and atomic refusal on missing
  target data. Full real-world/Qt/Windows runtime testing deferred.

## 0.3.0a64 — Original BE-aware Move Tool + BE transfer format isolation

- New opt-in **Move selection WITH original BE NBT (expert)** action
  in Qt6 Edit menu/Tool Palette. Entire 3D source and target are
  preflighted, including an overlapping translation and original
  BE/Section identities. No existing BE outside the source can be
  overwritten. Source-only cells clear to air, target receives
  original blocks and byte-preserved opaque BE data with only root
  x/y/z adjusted. One Undo/Redo action; Save still uses original
  region backup and risk confirmation.
- Corrected general `.nbt` structure import: it now refuses marked
  original-BE transfer files even when there are no BE records,
  preventing accidental use of the wrong import workflow.
- Added native two-BE overlap/failure/Save+Undo regression sources,
  plus Qt6 confirmation/navigation source and marked-file guard.
  Real-world runtime test matrix still deferred.

## 0.3.0a65 — Exact opaque-NBT Java BE-aware Clone Tool

- Added explicit native Java original-BE clone and 1–32 repeat-stamp
  tiling with frozen source snapshots, per-target native Section
  preflight, non-overwrite of existing destination BEs and
  deterministic last-copy-wins on duplicated target positions.
- Optional source-air skip and destination-air-only guards preserve
  protected BEs. Block/BE writes stage as one Undo/Redo operation,
  with root x/y/z relocation only and native Save/backup semantics.
  Planned work capped at 32,768 cells, 512 BE records, 8 MiB BE data.
- Dedicated Qt6 Edit-menu and dockable Tool Palette expert workflow;
  old block-only clone behavior remains unchanged.
- Added native disk-backed and offscreen Qt regression **source**.
  Full runtime validation, including modded machine/network semantics,
  remains deferred.

## 0.3.0a66 — Read-only 3D original/staged Java BE wireframes

- Introduced original Java BE metadata scanner with strict X/Z
  chunk identity, raw BE record identity and staged edit/deletion
  statuses, bounded to 5×5 chunks, ±32 Y and 256 boxes.
- Streaming Qt6 OpenGL renders cyan original, gold staged, red
  pending-delete BE wireframes through its existing selection line
  renderer, without decoding NBT during paintGL or rebuilding
  terrain meshes on camera orbit.
- View menu toggles the overlay; diagnostics dialog lists partial
  / unreadable chunks, exact status counts, navigable BE positions
  and a copyable report. Scan is cache-keyed by source session,
  camera chunk and Y, refreshed after native edits.
- Added disk-backed native and offscreen Qt **test source**.
  Full real-world GL/runtime matrix and broad MCEdit2 feature
  parity remain outstanding.

## 0.3.0a67 — Read-only multi-chunk original Block Entity search

- Added native Java BE exact namespaced-ID / wildcard search over a
  selected XYZ area or current chunk, strict 64-chunk cap, 256
  listed results and total hit counts. Native NBT record validation
  and original chunk identity checks prevent silently treating
  malformed reads as complete search results.
- Optional unsaved BE staged overlay identifies originals,
  modifications, additions and deletions without rewriting data.
  Missing chunks are recorded as **partial coverage**, not absent
  BEs. Qt6 Edit-menu and Tool Palette results allow exact XYZ
  navigation and a copyable report.
- Fixed 3D BE wireframes disappearing when there was no selection.
- Added native disk-backed and offscreen Qt regression **source**;
  broad real-world Windows/OpenGL runtime validation is pending.

## 0.3.0a68 — Cancellable indexed generated-world original BE search

- Uses original region-header `RegionIndex` to enumerate generated
  chunks across the active dimension; no empty chunk generation
  or world writes. One native read-only world instance per Qt worker.
- Integrates exact-ID or `*` BE search and a frozen snapshot of
  unsaved BE edits with progress after each eight chunks and
  cancel-at-chunk-boundary semantics. Up to 200,000 generated chunks,
  8,192 regions and 256 listed results (complete count maintained).
- Reports missing/corrupt region headers, invalid NBT chunks,
  orphaned staged edits, truncation and cancellation explicitly;
  partial results never imply all unlisted BEs are absent.
- Adds Qt6 Edit-menu / Tool Palette workflow, cancelable progress
  and read-only navigable results. Save/region restore are blocked
  while the worker scans; app exit first cancels running workers.
- Adds disk-backed native and offscreen Qt **regression source**.
  Full OS/Minecraft runtime testing and greater world-scale search
  coverage remain pending.

## 0.3.0a69 — Original Entity find across legacy and split regions

- Index **both** original old Java entity lists in terrain
  chunks and modern standalone `entities/*.mca` region files.
  Reuse original header indexing and opaque NBT entity decoder
  with strict available chunk identity/ID validation.
- Exact native entity ID or wildcard search, staged deletion,
  modification, creation and cross-chunk transfer status
  snapshot, capped generated-world scan with complete/
  incomplete coverage diagnostics and limited hit list.
- New independent read-only Qt6 worker, progress/cancel,
  navigable coordinate matches, copyable reports and source
  labels. Save/restore/close safety now covers Entity and
  Block Entity background scans.
- Added native disk-backed and offscreen Qt regression
  **test sources**; runtime verification of large real
  modded worlds and Windows/OpenGL remains outstanding.

## 0.3.0a70 — CI regression repair + private original world checks

- Enabled CI on the feature branch and added one exhaustive
  Python 3 top-level migrated `unittest` job (including native
  and Qt tests), rather than only a selected legacy list. Original
  Python 2 tests are not compiled as if migrated Python 3.
- First full run: 873 tests, five failures/two errors; all seven
  cases diagnosed/fixed. **Native safety fix:** destination-air
  clone masks never write atop BEs attached to air-state cells.
  Subsequent 873-test full run plus Python 3.12/3.13/3.14,
  Qt6 and Mesa OpenGL jobs passed.
- Audited two private user Java world ZIPs locally, using
  read-only sector/NBT parsing. Version 1343: 637 terrain
  chunks, 33 BEs. Version 3955: 2338 terrain chunks,
  1329 BEs, 155 separate Entity chunks, and 72 empty
  region-file placeholders; no populated chunk NBT errors.
- Added `scripts/validate_real_world_archives.py` for the real
  archives to be tested through the native backend without
  publishing ZIP contents, plus a synthetic archive validation
  regression for CI. Running this backend validator on the
  actual supplied ZIPs must occur in an environment with the
  full source tree and dependencies. GitHub Actions cannot
  directly access chat-uploaded ZIPs.
- Aligned packaged `pyproject.toml` and runtime version
  to 0.3.0a70. Real game/OS/3D editing parity tests remain
  outstanding.

## 0.3.0a71 — Native real 1.12.2 / 1.21.1 validation, empty-region fix

- Obtained current public native backend source through a source-only
  GitHub Actions artifact; user-supplied worlds stayed local/private.
  **Executed** full original chunk reads, bounded Section decode and
  indexed BE/Entity searches on both actual uploaded worlds.
- Verified 1.12.2: 637 terrain chunks / 33 BEs / 304 Entities.
  Verified 1.21.1: 2,338 terrain chunks / 1,329 BEs / 155 independent
  Entity chunks with 405 Entities. All original generated chunks
  decoded without failure.
- Fixed genuine indexed whole-world search false-INCOMPLETE from 72
  zero-byte `.mca` placeholders in the actual 1.21.1 world.
  `RegionIndex.empty` tracks these separately from nonempty
  malformed headers, while exact BE/Entity diagnostics preserve
  counts and completeness.
- On privately isolated world copies, tested native EditSession
  paint, Undo, Redo, Save, reopening, opaque original BE bytes and
  byte-identical original region backup restoration for both world
  versions; **all exercised operations passed**.
- Added a reusable guarded private-ZIP one-cell roundtrip runner,
  synthetic CI smoke and zero-byte placeholder regressions.
  Game loading, all mod interactions and exhaustive edit matrix
  remain unverified.

## 0.3.0a72 — Real native multi-region editing preservation matrix

- Extended private 1.12.2 and 1.21.1 native world copy testing
  from one cell to nine edited generated chunks / 26 affected
  coordinates per world, including an actual cross-chunk Line.
- Confirmed byte-for-byte preservation of **21 original BEs in
  1.12.2 and 303 in 1.21.1** across affected chunks, plus
  all non-Section original NBT, all unedited generated chunks
  in modified regions, and native exact region backups.
- Tested grouped Fill and Line Undo/Redo, staging without
  disk writes, native Save, fresh reopen, and full region-byte
  recovery from original backups, all on temporary copies.
- Committed reusable bounded private-ZIP multi-region runner
  and synthetic multi-region / BE-collision regressions to
  the existing Draft PR. Never commit real user world bytes.

## 0.3.0a73 — Actual Java modded Block Entity clone/move verification

- Ran the original 1.21.1 world with the current native backend
  for two real Block Entity sources and both Clone and Move:
  a modded Lootr barrel crossing an .mca region boundary and a
  5,146-byte original vanilla spawner BE crossing a chunk boundary.
- Verified exact opaque root-XYZ-only BE NBT relocation, original
  state restoration under Undo, repeat application under Redo,
  disk Save/reopen, original BE collision rejection and
  byte-exact region backup recovery.
- Verified original BEs elsewhere in both touched chunks and
  untouched original chunks in affected .mca regions keep their
  exact NBT bytes. Original user ZIP files were never rewritten.
- Confirmed native 1.12.2 numeric BE move/clone fails closed,
  without staging data, rather than pretending to support
  unsafe conversion.
- Added repeatable private-ZIP BE transfer runner and synthetic
  cross-region regression tests to the existing Draft PR.
  The full migrated CI suite passed 887 tests in 95 modules.

## 0.3.0a74 — Safe Java 1.12 native numeric BE clone/move

- Enabled guarded original Java Anvil numeric ID/meta BE-aware
  Clone/Move, including raw IDs up to 4095 and high Add nibbles,
  without replacing old blocks with guessed modern names.
- Save now validates paired original BE creation/removal and
  numeric block changes; ordinary legacy block-only writes
  still cannot overwrite BEs. Refuses malformed air-state BEs,
  absent chunks and occupied BE targets before staging.
  Old MCRegion format stays unsupported.
- Qt6 Edit menu enables the verified native numeric backend.
  Extended real ZIP audit to four actual 1.12.2
  `ebwizardry:bookshelf` clone/move tests (792-/771-byte
  original NBT), plus previously passing four 1.21.1 BE
  transfers. Original metadata and NBT, unedited chunks and
  region backups verified unchanged.
- New synthetic regression `test_legacy_be_transfer.py`
  exercises numeric ID 511/data 13 and legacy data safety.
  Full GUI/OS and mod semantic behavior are still not claimed.

## 0.3.0a75 — Native BE-aware mixed ordinary-block selection bug

- Corrected protected original BE guard: **no original BE + no
  proposed BE** means a normal block edit in an explicit BE-aware
  Clone/Move is permitted. Previously mixed selections could fail
  incorrectly when a normal block changed, including valid
  native 1.12/1.21 original Java worlds.
- Real-world two-version matrix: 1.12.2 cloned/moved **5 BEs**
  in 12/24 affected cells; 1.21.1's final committed selector
  cloned/moved **5 BEs** in 24/42 affected cells, with an additional
  real two-BE one-block overlapping move in each world. Exact opaque BE NBT relocation,
  one Undo/Redo group, native Save/reopen, original backup
  restore and unrelated generated chunk NBT byte equality
  verified on disposable world copies.
- Added a repeatable bounded private-world multi-BE runner,
  modern/legacy atomic transaction regressions and synthetic
  ZIP cross-chunk/overlap/missing-target tests. Unsupported
  mod-specific linkage and full GUI behavior remain unverified.

## 0.3.0a76 — Complete parallel original-world regression in one command

- Introduced a **single integrated EditSession + one Save** scenario
  mixing multi-BE move, normal block paint, safe dropped-item UUID
  clone and grouped move of two original entities. One Undo/Redo
  history stack is fully reversed and replayed before native
  Save/reopen and byte-exact original multi-region recovery.
- Same scenario passed on the real **1.12.2** original numeric
  format (old inline Entity list) and **1.21.1** named format
  (independent entities/*.mca), preserving original opaque BE
  and untouched original chunk NBT.
- Added a **one-command comprehensive six-suite matrix**
  `scripts/verify_native_matrix.py`, running two original private
  archives in parallel and collecting all suite results/errors
  into a consolidated JSON without uploading or editing source ZIPs.
  The full local 1.12.2 + 1.21.1 matrix passed **12/12**.
- Added full synthetic integrated-world and matrix orchestrator
  regressions to default GitHub Actions Python 3 discovery
  (100 migrated modules/903 tests passed). Long-term
  GUI migration and Minecraft in-game semantic testing
  remain separate goals.

## 0.3.0a77 — Multi-rule original-native BlockState replacement

- Added a Qt6 Edit menu **Batch replace native BlockStates…** command
  accepting up to 32 explicit source -> target rules, with exact-state
  versus native-ID matching, read-only preview and confirmation.
- Matching is simultaneous against the original staged state of each
  selected cell: transformations **do not cascade** between rules.
  Duplicate source patterns are rejected instead of applying ambiguous
  precedence, including legacy numeric metadata identity.
- Requires a backend with original Block Entity position detection.
  Skip & preserve protected BEs is enabled by default; a strict option
  rejects the entire operation on any matching changed BE.
- Validated native destination writes are staged through the existing
  all-or-nothing EditSession preflight, one Undo/Redo history entry and
  original Anvil region backups. Existing opaque BE NBT and unrelated
  chunk tags remain unchanged. No mod-specific semantic rewrite,
  lighting recalculation or Minecraft in-game load is claimed.
- Added native Anvil save/reopen/opaque-BE regression, non-cascading,
  duplicate refusal, atomic preflight rejection and offscreen Qt6
  dialog smoke coverage.

## 0.3.0a78 — Lossless legacy numeric Anvil original-BE structure files

- Extended the **explicit, marked MCEdit2 original Block Entity .nbt structure**
  export/import workflow to original **pre-1.13 numeric Anvil (.mca)**
  alongside previously supported named 1.13+ Anvil.
- Export preserves original numeric block ID (0..4095) and metadata (0..15)
  including unknown modded combinations as native opaque states. Original
  BE Compound payloads are byte-identical; only BE root x/y/z TAG_Int changes
  upon placement. No automatic DataFixer, registry mapping or mod link rewriting.
- Requires a known, exact source/destination DataVersion and native BE-safe
  backend, and refuses MCRegion (.mcr), existing destination BEs and unavailable
  chunks/Sections. All legacy BE **destinations must be original numeric
  air (ID 0, meta 0)** so removing transferred BE records remains safe
  even after native Save followed by Undo and another Save.
- Qt6 expert import/export warnings now disclose legacy numeric limitations.
  Add disk-backed synthetic legacy roundtrip/Undo-after-Save and refusal tests.
  Verified actual 1.12.2 (DataVersion 1343) and 1.21.1 (3955) original
  private ZIPs in isolated temporary copies: both export/import/Undo/Redo,
  save/reopen, Undo after Save, no original ZIP modification; not proof of
  Minecraft client or mod controller linkage correctness.

## 0.3.0a79 — Standard Java NBT structures with original Block Entity NBT (expert)

- Added independent **standard, unmarked Java .nbt** structure import/export
  with optional original Block Entity Compound NBT. The normal block-only
  structure path is unchanged and still rejects BE/entity-bearing files;
  the separately marked MCEdit2 original BE transfer remains separate.
- Standard file export strips only original BE root x/y/z TAG_Int fields
  (stored as target positions by the structure format), preserving all other
  original opaque NBT fields including unknown mod capabilities and arrays
  byte-exact. Import accepts all three old integer XYZ fields or none and
  canonicalizes only those coordinates in-memory. Source files stay unchanged.
- Reuses original strict native Anvil BE transaction preflight:
  **matching DataVersion**, Java **1.13+ named** palette (not legacy
  numeric or MCRegion), generated destination Sections, no existing BE
  collisions, one atomic Undo/Redo operation and original region backups.
  Standalone entities and passenger/UUID graphs are **refused**; unknown
  root/block fields, malformed BE NBT and oversized payloads fail closed.
- Qt6 File menu exposes **Import/Export vanilla Java structure with BE
  (.nbt; expert)** with explicit confirmation and modded-link warnings.
- Added synthetic native Save/reopen, Undo-after-Save, optional author,
  malformed BE, collision and Qt6 UI regressions. Locally validated against
  the user's private real 1.21.1/DataVersion 3955 ZIP in disposable copies;
  original ZIP stayed byte-identical. 1.12.2 numeric Java is intentionally
  refused by this standard route and retains its original-ID-specific
  transfer tool. In-game modded machine/network semantics are unverified.

## 0.3.0a80 — Sponge v2/v3 original Block Entity schematic transfer

- Added a separately confirmed **Sponge .schem v2 and v3 with opaque BE**
  expert import/export path without weakening the normal block-only importer.
  Sponge v2 flat BE properties, Sponge v3 nested BE Data Compound, Id and
  relative Pos TAG_Int_Array[3] are handled distinctly. Unknown modded
  properties remain original binary NBT tags.
- Both formats retain exact native block-state identity, verify a known
  Java 1.13+ DataVersion and reject unknown entity/biome data and unsafe
  nonzero offsets via the existing strict Sponge decoder.
- Placement reuses original native Anvil BE-safe preflight, no existing BE
  overwrite, generated Sections, one Undo/Redo entry and region backups;
  absolutely no automatic datafixing, UUID/machine-network repair or
  legacy numeric-to-named conversion.
- Added Qt6 File menu commands for v2/v3 BE-aware .schem import/export
  with explicit warnings; disk-backed native synthetic regression for
  Sponge v2/v3 encode/decode, save/reopen, Undo-after-save and
  malformed/unsafe data refusal.
- CI is intentionally deferred while the user batches five feature
  commits. This is feature commit 1 of 5 and is marked [skip ci].

## 0.3.0a81 — Legacy Alpha .schematic original numeric BE expert transfer (2/5)

- Added explicitly marked legacy Alpha .schematic export/import carrying
  full 0..4095 original block ID + 0..15 metadata and original opaque
  TileEntities, without mapping unfamiliar modded IDs to modern blocks.
- Alpha .schematic lacks a trustworthy Minecraft DataVersion. For safe
  lossless BE transfer, this expert file requires an MCEdit2DataVersion
  TAG_Int; arbitrary legacy schematic BE input without the marker is
  rejected rather than guessed. The ordinary block-only importer stays
  strict and does not accept BE-bearing files.
- Export rewrites BE root xyz coordinates from absolute to local and
  saves all non-coordinate original NBT bytes unchanged. Import checks
  exact destination DataVersion and preflight; all legacy BE targets must
  be ORIGINAL numeric 0:0 air. No MCRegion .mcr, external BE overwrite,
  missing Section, entity/ticks transfer, network/UUID repair or cross-
  version conversion.
- Added Qt6 expert File menu import/export options and targeted synthetic
  native Anvil save/reopen, post-save Undo and failure fixtures.
- No CI for this commit [skip ci], per five-feature batch request.

## 0.3.0a82 — Non-destructive source-air mask for all expert BE imports (3/5)

- Added opt-in skip_air=True to original native BE structure placement,
  Sponge v2/v3 BE, Java .nbt BE and legacy Alpha .schematic BE wrappers.
- Skip ONLY exact source vanilla minecraft:air/cave_air/void_air without
  properties, or for old numeric Anvil only original ID=0 and metadata=0.
  Never infer modded "air-like" blocks; all source BE identities are
  validated first. No write is staged for skipped cells, regardless of
  destination, avoiding unwanted erasure by air in partial pastes.
- The Qt6 File menu has a checkable "BE imports: skip exact source air"
  toggle used by each expert importer. Default off preserves earlier behavior.
  Real non-air cells and BEs continue full native preflight; one Undo,
  Save/reopen and original region backups unchanged.
- Added synthetic modern and numeric legacy Anvil Undo/Save regressions,
  custom air-name non-skipping, strict invalid option refusal.
- CI intentionally deferred while batching five feature commits [skip ci].

## 0.3.0a83 — Original-air-only destinations for all expert BE pastes (4/5)

- Added optional only_replace_air target filter for marked .nbt, standard
  Java .nbt, Sponge v2/v3 .schem and legacy Alpha .schematic BE imports.
- Only the original, EXACT vanilla air/cave_air/void_air named identities
  or original legacy numeric 0:0 count as air. BOTH original Section
  and current staged block must be air before the paste writes that cell.
  Destination BE conflicts still fail the whole operation, never overwrite.
  Non-air targets are skipped, preventing accidental replacement of
  existing constructions; no skipped destination is recorded in Undo.
- Qt6 adds a separate checkable "BE imports: only write into original
  destination air" toggle, combinable with the strict source-air mask.
  Existing modes remain unchanged unless the user enables the option.
- Added synthetic modern and pre-1.13 numeric Anvil placement, backup,
  Undo-after-Save, staged occupancy and conflict refusal regressions.
- No CI is run for this batch commit [skip ci].

## 0.3.0a84 — Read-only exact-preflight BE paste preview (5/5)

- Split original BE structure placement into a **shared immutable planning**
  stage and a separate commit stage, so preview and Apply use the exact
  same DataVersion, existing Section, numeric/original-state and
  Block Entity collision checks. Apply RE-SCANS after user confirmation
  instead of trusting potentially stale preview results.
- Read-only summary counts total original source blocks, actual changed
  cells, new BE records, strict source-air cells skipped and occupied
  target cells skipped. The preview NEVER mutates world files, stages
  unsaved edits or changes the Undo/Redo history.
- Four expert GUI BE import paths (.nbt marked, .nbt vanilla, .schem
  v2/v3, Alpha .schematic) show actual native preflight summary before
  confirmation. Unsafe operations are rejected before the dialog.
- Added modern and numeric legacy Anvil test fixtures for preview vs
  actual commit parity, air-mask counts, collisions, unavailable chunks
  and DataVersion refusals.
- This is the 5th feature commit of the requested batch. Full CI is
  now enabled to validate all five features together.

## 0.3.0a85 — BE-aware pending queue atomic grouped placement (1/5)

- Native multi-file BE structure queue backend now preflights every original
  Anvil destination together and places them in ONE Undo step.
- The preview is read-only; BE collision, unavailable chunk/Section, duplicate
  structure footprint, mismatched DataVersion and combined budgets fail closed.
- No implicit last-writer-wins when queued original BE structures overlap.
- Synthetic native Anvil Save/reopen and Undo-after-Save regression coverage.
- Local private 1.21.1 world verified two BE targets and one Undo, without
  modifying the original ZIP. CI intentionally deferred [skip ci].

## 0.3.0a86 — Four-format opaque BE pending file loader (2/5)

- Explicitly select marked original .nbt, standard Java .nbt, Sponge v2/v3
  .schem or version-tagged numeric Alpha .schematic; never silently fall back
  to block-only import for any Block Entity NBT.
- Enqueue only on exact Java Anvil/DataVersion matches and safe integer XYZ;
  malformed mask flags or wrong extensions are rejected before world edits.
- Synthetic modern/legacy fixtures and private 1.21.1 read-only checks.
- CI deferred per five-feature batch [skip ci].

## 0.3.0a87 — One transaction for mixed block-only and BE queues (3/5)

- Mixed structure groups can contain ordinary block-only and BE-aware files;
  all destinations are preflighted together and staged in one native Undo.
- Mixed queues fail closed on any overlapping original footprint, invalid
  backend writes, BE collisions, missing Sections, and unconfirmed version
  mismatches. Existing pure block-only queue last-wins behavior is unchanged.
- Read-only preview reports total block and BE changes. Native Save/reopen,
  Undo-after-Save tests, and private real 1.21.1 mixed placements verified.
- CI deferred until feature 5 [skip ci].

## 0.3.0a88 — Qt6 pending import dock: BE-aware mixed queue (4/5)

- The original pending structure dock now queues explicit expert BE files
  alongside existing block-only files without changing the default loader.
- Qt6 format picker requires named expert BE codec. Per-entry XYZ and source/
  destination air-mask controls remain editable before Apply.
- Mixed BE+plain queues show the complete read-only native plan before
  confirmation; commit revalidates, stages ONE Undo, and clears only on success.
- Added Qt6 offscreen GUI regressions for original BE and mixed imports.
- CI deferred until fifth feature [skip ci].

## 0.3.0a89 — Pending BE file/world provenance and stale-queue refusal (5/5)

- Expert BE queued imports now carry a SHA-256 fingerprint of the exact file
  and the identity of the original open world. Loading verifies that the
  file stayed unchanged during parsing; preview and Apply recheck the source.
- A changed/deleted/oversized file or a reopened/different world is refused
  before any native writes. In-memory explicit transfers remain supported.
- Qt6 pending panel displays the source fingerprint for selected BE entries;
  read-only mixed previews and single Undo continue to use exact preflight.
- Added native Anvil tests for source edits, deletion and different world
  identity. Feature 5 of 5: run one complete CI matrix after this commit.

## 0.3.0a90 — Major feature 1/5: bounded cross-chunk entity selection clipboard

- Full **independent standalone-entity 3D selection clipboard**: scan up
  to 256 existing Java Anvil chunks, capture up to 128 independently
  cloneable original dropped-item / experience-orb NBT records, preserving
  opaque ItemStack and modded Compound fields.
- Native multi-chunk paste supports one or many separate original source
  chunks and destination entity chunks, both 1.12 numeric and modern
  Anvil entity storage without regenerating missing chunks.
- Each copied entity obtains an independent UUIDv4. Only original Pos and
  UUID bytes are rewritten; other raw NBT members remain byte-identical.
  Source entities are never deleted, and each repeat paste gets distinct
  UUIDs. Existing entity region data and terrain NBT are left untouched
  until explicit Save with automatic original region backup.
- Memory-only preview checks original source snapshots, correct source
  and destination chunk, existing target NBT, original duplicate UUIDs,
  batch limits, world instance/dimension and DataVersion; no staging or
  UUID allocation during preview. Commit rechecks then stages all additions
  as **one Undo/Redo**. Entity structural history is cleared on Save as
  original member indices change; backup restoration remains supported.
- Added Qt6 selection Copy + XYZ-offset Paste expert actions, preview
  confirmation and new-world/dimension clipboard clearing. Only
  independent vanilla item/orb entities are supported: mobs, passengers,
  leashes, ownership references and unknown modded entity graphs are
  deliberately refused. No arbitrary schematic entity compatibility.
- New disk-backed dual-chunk synthetic tests include UUID/opaque NBT
  fidelity, grouped Undo/Redo, Save/reopen/backup, cross-chunk paste,
  repeated pastes, missing destination, stale source, wrong world,
  unsupported entities and GUI dialog interactions.
- **Major feature 1 of 5** in the new CI batch. No CI until all five
  substantial feature commits are complete. [skip ci]

## 0.3.0a91 — Major feature 2/5: standard Java structures with standalone entities and BEs

- Added separate **unmarked standard Java .nbt import/export** for one
  selection containing native blocks, opaque original Block Entities and
  standalone dropped items / XP orbs, all in a single expert file.
  Normal block-only import and BE-only import remain fail-closed for
  nonempty entities, never silently discard original entity contents.
- Export stores vanilla entities[] records with relative TAG_List[Double]
  pos, relative TAG_List[Int] blockPos and original entity NBT Compound.
  Only existing entity Pos is rewritten relative; unknown opaque ItemStack
  and mod-specific members remain byte-exact. Files retain original UUIDs
  and MUST NOT be pasted by vanilla structure blocks or unrelated tools,
  to avoid duplicate UUID identities.
- Import accepts **Java 1.13+ named Anvil with exact original DataVersion**.
  Every entity MUST be a known independent vanilla item/XP orb with
  complete original Pos+UUID and no linked owner/passenger/reference graph.
  Unsupported/missing entities, other wrapper members and malformed
  coordinate/UUID tags fail closed. Import relocates Pos and issues an
  independent UUIDv4 per new entity, preserving other NBT bytes.
- Mandatory read-only atomic preflight spans **all blocks, BEs and entities**:
  target Sections, existing entity chunks, BE collisions, entity UUID
  conflicts, size and 128-entity/8-MiB budgets. Missing chunks are not
  generated. Commit revalidates then stages ONE Undo transaction; entity
  structural history clears after Save, with original region backups.
- Added two Qt6 File menu expert options with explicit high-risk warnings,
  3D selection export, target-origin import and native counts/previews.
  Includes disk-backed synthetic region tests for encode/decode, opaque NBT,
  saved world reopen, backup/Undo/Redo, repeated fresh UUIDs, BE conflict,
  version mismatch, missing entity storage and GUI behaviors.
- No generic mob/vehicle/passenger cloning, structure entity graphs,
  cross-dimension edits, DataFixer mappings or direct vanilla-game
  structure-placement safety is asserted.
- **Major feature 2 of 5** in current batch; CI intentionally deferred
  [skip ci], to be run after major feature 5.
