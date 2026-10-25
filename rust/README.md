# MCEdit2 Rust + iced workspace

The root of this branch is the native rewrite, not the original Python application. The old implementation is preserved in the repository's `master` history and in the separate Python 3 Draft PR. This workspace contains no Python runtime, Cython or Qt dependency.

## Crates

| Crate | Purpose |
| --- | --- |
| `mcedit2-world` | Bounds-checked Java Minecraft world/NBT/region formats, original-state editing, transaction and schematic helpers |
| `mcedit2-iced` | iced 0.14 desktop, wgpu 3D view and selection, streaming/chunk meshing, vanilla resource-pack models and multi-atlas textures |
| `mcedit2-cli` | Headless source world inspection |

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
