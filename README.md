# MCEdit2 — native Rust rewrite (work in progress)

This branch is a **Rust + iced** rewrite of the Minecraft Java world editor. The Rust workspace is under [`rust/`](rust/); **Python and Qt are not runtime or build dependencies**.

The original MCEdit2 source remains available on the [`master` branch](https://github.com/yanxiatao/mcedit2/tree/master). The independent Python 3 / Qt migration remains in [Draft PR #1](https://github.com/yanxiatao/mcedit2/pull/1). This Rust branch is tracked by [Draft PR #2](https://github.com/yanxiatao/mcedit2/pull/2); it is not release-ready.

本批共享区块保存与固定工作区布局的实现和验收进度见 [进度记录](rust/PROGRESS.md)，存档 / 视图标签与工具页面排列见 [界面说明](rust/UI.md)。整体重写与完整渲染验收仍在进行。

## Run the native editor

Install a current stable Rust toolchain, then:

```sh
cd rust
cargo run -p mcedit2-iced
```

On launch, choose an **extracted Minecraft Java world directory** containing `level.dat`. The default workspace is a spectator-style 3D viewer with mouse-look, keyboard flight, streamed source chunks and Java resource-pack texture loading. Advanced native tools provide world inspection and guarded editing operations.

A CLI is also available:

```sh
cd rust
cargo run -p mcedit2-iced -- --inspect "/path/to/Minecraft/saves/World"
```

产品只有 `mcedit2.exe` 一个入口：默认打开 GUI，`--inspect` 执行终端检查，`--plugin-host` 是程序自行启动的隔离插件模式。开发和测试统一复用 `rust/target/debug/mcedit2.exe`，不需要伴随 CLI 或 Python 程序。 The optional Windows ZIP packaging workflow is explicitly gated and is **not** run on ordinary source changes.

主三维已增加实体、物品、方块实体及计划更新的位置图层，可后台读取并只读点选原始位置。定位框与实体模型分开，完整模型及其他迁移缺口继续记录在 [迁移表](rust/MIGRATION.md)。

## Architecture and current scope

- **`mcedit2-world`** — native Java `level.dat`, Alpha/MCRegion/Anvil/NBT readers, original block states, guarded edit staging and loss-conscious Region transactions.
- **`mcedit2-iced`** — iced/wgpu 3D spectator viewport, asynchronously streamed terrain, source-aware picking and multi-page, mixed-resolution texture-array atlas; 2D inspector and advanced tools.
- **命令行模式** — 同一个 `mcedit2 --inspect` 检查解压后的 Java 世界，不创建 GUI。
- **`mcedit2-plugins` / `mcedit2-python`** — 插件发现、启停、独立 RustPython 命令进程，以及经过原生校验的可撤销预览；兼容范围见[迁移记录](rust/MIGRATION.md)。
- **`test_files/`** — non-Python original-format world and schematic fixtures retained for native format regression tests.

**Important limitations:** not all Minecraft releases, snapshots, modded blocks, rendering effects, Block Entities, entities, or original MCEdit2 editing functions have been verified or fully ported. The 3D far horizon uses progressively reduced detail, not game-equivalent complete simulation. Do not edit your only copy of a world.

当前测试采用标准 Rust harness，命令及需要额外设备 / 实例资源的检查见[测试说明](rust/TESTING.md)。历史验收结果见[进度记录](rust/PROGRESS.md)，完整迁移和真实存档验收仍有缺口。另见 [Rust 工作区说明](rust/README.md)与[格式和保存约定](rust/COMPATIBILITY.md)。

## Licensing and history

The original MCEdit2 BSD license is preserved in [LICENSE.md](LICENSE.md); the independent Rust crates currently declare their license in the workspace manifests. Historical source and Python-specific developer instructions are recoverable via Git history and the separate `master` / Python migration branches, rather than shipped as part of the Rust source tree.
