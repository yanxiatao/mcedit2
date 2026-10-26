# MCEdit2 — native Rust rewrite (work in progress)

This branch is a **Rust + iced** rewrite of the Minecraft Java world editor. The Rust workspace is under [`rust/`](rust/); **Python and Qt are not runtime or build dependencies**.

The original MCEdit2 source remains available on the [`master` branch](https://github.com/yanxiatao/mcedit2/tree/master). The independent Python 3 / Qt migration remains in [Draft PR #1](https://github.com/yanxiatao/mcedit2/pull/1). This Rust branch is tracked by [Draft PR #2](https://github.com/yanxiatao/mcedit2/pull/2); it is not release-ready.

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
cargo run -p mcedit2-cli -- "/path/to/Minecraft/saves/World"
```

Windows binaries use `mcedit2.exe` and `mcedit2-cli.exe`. The optional Windows ZIP packaging workflow is explicitly gated and is **not** run on ordinary source changes.

## Architecture and current scope

- **`mcedit2-world`** — native Java `level.dat`, Alpha/MCRegion/Anvil/NBT readers, original block states, guarded edit staging and loss-conscious Region transactions.
- **`mcedit2-iced`** — iced/wgpu 3D spectator viewport, asynchronously streamed terrain, source-aware picking and multi-page, mixed-resolution texture-array atlas; 2D inspector and advanced tools.
- **`mcedit2-cli`** — headless audit/inspection of extracted Java worlds.
- **`test_files/`** — non-Python original-format world and schematic fixtures retained for native format regression tests.

**Important limitations:** not all Minecraft releases, snapshots, modded blocks, rendering effects, Block Entities, entities, or original MCEdit2 editing functions have been verified or fully ported. The 3D far horizon uses progressively reduced detail, not game-equivalent complete simulation. Do not edit your only copy of a world.

Source changes after the last explicitly authorized tests may be **uncompiled and untested**. No automatic CI runs or new packages are implied by this README. See [Rust workspace guide](rust/README.md) and [format and safety contract](rust/COMPATIBILITY.md).

## Licensing and history

The original MCEdit2 BSD license is preserved in [LICENSE.md](LICENSE.md); the independent Rust crates currently declare their license in the workspace manifests. Historical source and Python-specific developer instructions are recoverable via Git history and the separate `master` / Python migration branches, rather than shipped as part of the Rust source tree.
