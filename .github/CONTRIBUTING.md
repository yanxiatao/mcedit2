# Contributing to the MCEdit2 Rust rewrite

This branch contains the native Rust + iced application. Use `rust/Cargo.toml` as the workspace root; do not submit Python, Qt, Cython, pip or legacy PySide installation changes against this branch. The historical implementation remains in `master` and the separate Python 3 Draft PR.

For an issue or pull request, specify the Minecraft Java release / DataVersion, OS, Rust toolchain version, steps to reproduce, expected and observed behavior, and any relevant diagnostics. State whether the failure concerns original NBT reading, block editing, spectator rendering, textures, or save transactions.

Avoid uploading private saves, world identities, launcher credentials or personal data to public issues. A minimal sanitized fixture is preferred. All source edits must preserve unrelated Block Entities and opaque modded NBT; no creation of missing chunks or conversion to invented block IDs.

CI and Windows packaging are manually authorized workflows. Ordinary code commits should not change the trigger sentinels.
