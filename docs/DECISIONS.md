# DECISIONS — RustShot

Every non-obvious choice + a one-line rationale. Newest at top within each phase.

## Global

- **Clean-room.** No Screenpresso code, resources, icons, strings, or layouts are copied. We
  reproduce *capabilities* only, using the RE analysis as architecture/UX inspiration.
- **Toolchain pinned via `rust-toolchain.toml`** (stable, MSVC, x64). Verified on this box:
  Rust 1.98.1, VS2022 BuildTools MSVC 14.44, Windows SDK 10.0.26100. Builds + links + runs.
- **Crate versions verified against crates.io on 2026-10-22** and centralised in the workspace
  `Cargo.toml`. Notables vs the prompt's indicative versions: `windows` 0.62.2 (not 0.58),
  `windows-capture` 2.0.1 (not ~1.5), `tray-icon` 0.26, `global-hotkey` 0.9, `eframe` 0.37.
  Exacts captured in `Cargo.lock`.
- **No `panic = "abort"`** in release: we keep unwinding so panics on worker threads can be
  caught and surfaced as a friendly dialog (§8 "no panic reaches the user").
- **Edition 2021** (not 2024): the prompt allows either; 2021 is the broadly-stable choice and
  nothing here needs 2024-only features. Revisit if a dependency requires 2024.

## M0 — Scaffold

- **Workspace with 5 crates** exactly per the prompt's layout. `recorder` and the Windows
  capture backend start as typed stubs so the workspace stays green before M1/M3.
- **Config format: TOML** at `%APPDATA%\RustShot\config.toml` via `directories`. Serde with
  `#[serde(default)]` on every field so older/partial config files load forward-compatibly.
- **Single-instance: named mutex** (`CreateMutexW` + `GetLastError == ERROR_ALREADY_EXISTS`)
  via the `windows` crate — the prompt's recommended approach, no extra dependency.
- **Dev CLI** (`clap`) lives in the app binary behind a `--dev <cmd>` subtree so GPU/display
  paths can be exercised on a real desktop without the full tray UX. Running `--dev` does not
  start the tray.
- **Hotkey model in `config`** as plain data (`mods: Vec<Modifier>`, `key: String`) so parsing
  and conflict detection are pure and unit-testable, independent of `global-hotkey`.
