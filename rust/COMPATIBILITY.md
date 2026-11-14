# Minecraft Java compatibility contract — Rust + iced rewrite

2026-11-02 启动脚本源码补充：`-eval` 使用 RustPython 及实际原生世界会话，支持后台执行、整组编辑发布、每维度一次历史、取消、源修订核对、实时日志及成功后排队的显式保存。`-view` 禁止修改和保存，`--inspect` 不与 `-eval` 同时使用。独立结构启动上下文、旧 Qt / NumPy 及完整旧对象接口仍未闭合；新增路径待完整迁移后的最终统一验证，不宣称原版脚本完全兼容。

2026-11-02 源码补充：已接入运行日志文件与共享底栏，`-log` 选择原生 wgpu 图形诊断，`-v/-vv/-debug` 控制对应级别；支持级别 / 来源筛选、重复计数、暂停 / 跟随、完整复制和保留记录快照导出。新增路径尚未编译、测试或开窗，留待完整迁移后的统一验证；后文此前的验证记录保留其原时间与范围。

当前状态说明（2026-10-27）：工作区已收束为 25 个类别测试入口，默认 22 类通过、3 类本机环境验收忽略，GPU 与本机实例类别另已显式通过。测试口径与运行见[按类别测试说明](TESTING.md)。已提供独立 RustPython 命令插件、四视图、原始 NBT 文档编辑内核及受保护的单 Region / 多 Region 保存组。资源加载使用可调整内存预算，Mod、启用资源包和来源层数不会按固定数量截断；自由视角与普通世界等轴共享流式区块几何、材质及源拾取，近景读取完整高度；方块信息可读取资源语言表中的中英文名称并保留原始 ID / 状态；手动限定窗口可在三种视图间保持，并支持源网格点选和拖动框选；GPU 控件等待后台网格时自行安排重绘，中断后的部分流式结果可继续完成；等轴 Canvas 已改为带深度的逐像素材质绘制；主三维已提供原始实体 / 方块实体 / 计划更新的位置图层及只读点选，定位框与实际实体模型分开；实体静态身体已扩展至准确 1.12.2 / 1.20.1 / 1.21.1 的源版本绑定、普通幼年变换及部分 NPC / 玩家 / 专用生物，结构预览复用相同版本目录；其他实体、姿态和装备仍有缺口；支持声明式组合模型、根变换、现代箱子、16 色床及原色 / 16 色潜影盒的静态实体外形，其他专用渲染器仍有缺口。请以[中文重写进度记录](PROGRESS.md)区分当前实现、历史里程碑和未完成的兼容性验收。测试通过不代表表中各版本均已通过游戏内读写验证。

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
原生写入仅对已实现并验证的布局和操作开放，保留原始不透明 NBT 并使用备份与恢复日志。
Alpha 和外部 `.mcc` 写入、光照/高度更新及逐版本游戏重载仍未完成验收；未知布局必须拒绝写入。

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
- -view：严格只读，禁止暂存及写盘；不影响读取和导航。
- One or multiple positional files/directories: recognized. Original
  world paths, level.dat and level.dat_old are accepted by the reader.
- -eval CODE：成功打开世界后执行一次；`session / self` 使用原生数据上下文，
  修改整组暂存，`session.save()` 请求在脚本成功后保存。接口与差异见
  `plugins/README.md`，不能据参数解析声称完整 CPython / Qt 兼容。
- Original .schematic and other files can be positional arguments:
  command parsing does not mean those formats have an importer yet.

Therefore "fully compatible CLI" is currently a contract and a
regression target, NOT a completed claim. No silent arg ignoring.

## Rust 开发验证

本机已具备 Rust 和有效依赖缓存。开发、测试和本机运行固定复用 `rust/target/debug`、`CARGO_INCREMENTAL=0` 与同一套 Cargo 目标 / 依赖特性图，一整批实际实现结束后统一编译：

```powershell
$env:CARGO_INCREMENTAL='0'
cargo test --offline -j 1 -p mcedit2-world -p mcedit2-iced --bin mcedit2 --test nbt_document --test runtime --no-run
```

只在已有类别程序中过滤本批受影响的类别，失败后只复验修复范围；不例行全量测试、全工作区格式化或重建 release。仅用户明确要求发布时构建 release。当前通过情况及实际窗口 / 游戏重载边界分别见 `PROGRESS.md`、`TESTING.md`。产品只有一个 `mcedit2` 入口，GUI、命令行和插件宿主通过参数选择。真实用户世界不提交到仓库。

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

`-view` 保持严格只读。`-eval` 已接入 RustPython 原生数据上下文；完整旧接口兼容仍未完成，新增路径待最终统一验证。

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

### Native block-only clipboard

Versioned `.mcedit2clip` provides same-world block-state source capture,
recoverable clipboard import, safe destination preview and Undo. It is NOT
a Minecraft schematic file and contains no opaque Block Entity/entity NBT.
Source or destination Block Entity collisions refuse the entire action.

### Sponge v2 block-only .schem import/export

Named-state Sponge v2 GZip NBT with exact target DataVersion supports
lossless block-state-only transfer. Entities, BlockEntities, biomes and
nonzero schematic Offset cannot be carried and refuse import, rather than
being stripped silently. v3, full structure NBT and conversions remain
unsupported.
