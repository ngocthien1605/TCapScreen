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
- **Dev CLI uses a `dev` subcommand** (`rustshot dev grab-monitor 0 out.png`) rather than the
  prompt's literal `--dev` prefix — clap models nested subcommands cleanly this way. Same intent.

## M1 — Capture engine (stills)

- **Raw WGC via `windows` 0.62 for `grab()`, not `windows-capture`.** We control stride, DPI,
  threading, and teardown directly; the WinRT projections are stable. The `windows-capture`
  crate's value is its Media-Foundation `VideoEncoder` (recording), so its evaluation is
  **deferred to M3** — and gated on whether its encoder accepts externally-supplied frames so
  `CaptureBackend` stays the seam. The prompt explicitly permits this fallback.
- **Per-grab D3D device + capture item + frame pool.** Keeps `WindowsBackend` holding no COM
  state (trivially `Send`); costs tens of ms, inside the 150 ms still budget. Streaming (M3)
  will keep a device alive instead.
- **Per-Monitor-V2 DPI awareness set programmatically** at the first line of `main()`
  (`SetProcessDpiAwarenessContext`), before the tray's hidden window exists. A manifest-based
  declaration is more robust and is deferred to M6/M7 with the icon resource.
- **WinRT apartment:** `RoInitialize(RO_INIT_MULTITHREADED)` in `WindowsBackend::new()`;
  S_FALSE / RPC_E_CHANGED_MODE tolerated. The free-threaded frame pool needs no DispatcherQueue.
- **Frame delivery:** `CreateFreeThreaded` pool + a `FrameArrived` handler that signals an mpsc
  channel; the grab waits on the channel with a 2 s hard deadline, then `TryGetNextFrame`. No
  busy-poll, no hang.
- **Stride = staging texture `RowPitch`** (may exceed `width*4`); `imaging::frame_to_rgba`
  honours it. On this dev box 1920×1080 yields `RowPitch == width*4` (no padding), so the
  padding path is exercised by **unit tests**, not this hardware.
- **Region/Window/VirtualDesktop** share the same `grab_item` pipeline: Region = grab its
  monitor + `Rect::to_local`/`clamp_to` + `Frame::crop`; VirtualDesktop = grab each monitor and
  `Frame::blit_into` a bounding-box buffer. Wired now; exercised from the UI in M2.
- **Near-black (DRM) detection** is a pure `imaging::is_near_black` heuristic; the app *warns*
  but still saves — a legitimately black screen is still a valid capture.
