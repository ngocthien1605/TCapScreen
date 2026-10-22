# RustShot

A lightweight, tray-resident **Windows screenshot and screen-recording** app written in Rust.
It does two things well: capture **screenshots** (region, window, monitor, all monitors) and
**record the screen** to MP4 (H.264 + AAC) or animated GIF — all driven from the system tray
and global hotkeys.

> Clean-room implementation inspired by the *capabilities* of Screenpresso. It contains no
> Screenpresso code, assets, UI, or branding.

## Features

- **Screenshots** — interactive region selection with a frozen-frame dimmed overlay and
  rubber-band, window picker with highlight, single-monitor, and all-monitors capture.
- **Flyout** — a non-modal preview after each capture with quick actions: Open, Open Folder,
  Copy, Copy Path, Delete.
- **Clipboard** — copy captures straight to the clipboard as an image.
- **Screen recording** — continuous Windows Graphics Capture → Media Foundation **H.264/MP4**,
  with constant-frame-rate pacing, pause/resume, and a recording toolbar.
- **Audio** — optional **system loopback** and **microphone** capture (WASAPI), mixed and muxed
  as **AAC** into the MP4, with sample-clock A/V sync.
- **GIF export** — record directly to an optimized animated GIF (downscaled, bounded by
  duration/FPS caps).
- **Global hotkeys**, a **Settings** dialog, **run-at-startup**, Per-Monitor-V2 **DPI awareness**,
  and single-instance behavior.

## Default hotkeys

| Action                | Default shortcut            |
| --------------------- | --------------------------- |
| Region screenshot     | `PrintScreen`               |
| Window screenshot     | `Ctrl` + `PrintScreen`      |
| Monitor screenshot    | `Alt` + `PrintScreen`       |
| All-monitors capture  | `Shift` + `PrintScreen`     |
| Start region recording| `Ctrl` + `Shift` + `R`      |
| Stop recording        | `Ctrl` + `Shift` + `S`      |

All hotkeys are configurable in Settings. If another app already owns a combination (e.g. a
screenshot tool holding `PrintScreen`), RustShot degrades gracefully and logs a warning.

## Requirements

- Windows 10 1903+ / Windows 11 (x64). Uses Windows Graphics Capture, Direct3D 11, Media
  Foundation, and WASAPI.
- To **build**: Rust 1.90+ with the `x86_64-pc-windows-msvc` toolchain, and the MSVC build tools
  (Visual Studio 2022 + Windows SDK).

## Build & run

```sh
# Debug build + run to tray
cargo run

# Release build
cargo build --release
# -> target/release/rustshot.exe
```

### Quality gate

Every change must pass the same checks CI runs:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
cargo test --workspace
```

Supply-chain policy (licenses, advisories, sources) is enforced with
[`cargo-deny`](https://github.com/EmbarkStudios/cargo-deny):

```sh
cargo install cargo-deny   # once
cargo deny check
```

## Developer / diagnostic CLI

The binary is a GUI app, but `rustshot dev <cmd>` exposes headless diagnostics (these do **not**
start the tray). When launched from a terminal it attaches to the parent console so output is
visible. Examples:

```sh
rustshot dev config                       # print the loaded (repaired) config + its path
rustshot dev paths                        # print all resolved RustShot paths
rustshot dev list-monitors                # enumerate monitors
rustshot dev list-windows                 # enumerate capturable windows
rustshot dev grab-monitor 0 shot.png      # full-monitor PNG
rustshot dev grab-all desktop.png         # whole virtual desktop
rustshot dev record-monitor 0 5 clip.mp4  # record monitor 0 for 5s to MP4
rustshot dev record-gif 0 3 clip.gif      # record monitor 0 for 3s to GIF
rustshot dev probe-mp4 clip.mp4           # decode back and report size/sample count
```

## Packaging

- **Portable zip:** `scripts/build-portable.ps1` → `dist/RustShot-<version>-portable-x64.zip`.
- **MSI installer:** WiX v3 source in `wix/main.wxs` (requires the WiX Toolset to build).

See [`docs/PACKAGING.md`](docs/PACKAGING.md) for details.

## Project layout

```
rustshot/
├─ crates/
│  ├─ capture-core/   # capture traits/types + Windows (WGC/D3D11) backend
│  ├─ recorder/       # Media Foundation H.264 encoder, WASAPI audio, muxing, GIF
│  ├─ imaging/        # crop, PNG/JPEG/GIF/ICO encode, thumbnailing, auto-naming
│  ├─ config/         # settings schema (serde), load/save, defaults, hotkey model
│  └─ app/            # BIN `rustshot`: tray, hotkeys, overlay, flyout, toolbar, settings, dev CLI
├─ docs/              # PLAN, DECISIONS, PROGRESS, QA-CHECKLIST, USER-GUIDE, PACKAGING
├─ wix/               # WiX MSI source
└─ scripts/           # packaging scripts
```

## Documentation

- [User guide](docs/USER-GUIDE.md)
- [Packaging](docs/PACKAGING.md)
- [Development plan](docs/PLAN.md) · [Design decisions](docs/DECISIONS.md) · [Progress](docs/PROGRESS.md) · [QA checklist](docs/QA-CHECKLIST.md)

## License

[MIT](LICENSE)
