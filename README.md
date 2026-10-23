# RustShot

A lightweight, tray-resident **Windows screenshot and screen-recording** app written in Rust.
It does two things well: capture **screenshots** (region, window, monitor, all monitors) and
**record the screen** to MP4 (H.264 + AAC) or animated GIF — all driven from the system tray
and global hotkeys.

> Clean-room implementation inspired by the *capabilities* of Screenpresso. It contains no
> Screenpresso code, assets, UI, or branding.

**Latest release: [v0.1.7](https://github.com/ngocthien1605/rustshot/releases/latest)** — a
portable zip, no installer. How to use it: **[Manual Guide](docs/MANUAL-GUIDE.md)** ·
**[Hướng dẫn sử dụng (Tiếng Việt)](docs/MANUAL-GUIDE.vi.md)**.

## Download & install

1. Download `RustShot-<version>-portable-x64.zip` from the
   [latest release](https://github.com/ngocthien1605/rustshot/releases/latest) (a `.sha256`
   file next to it holds the checksum).
2. Unzip it to a folder that will stay put and run `rustshot.exe`. No admin rights needed.
   The exe is not code-signed yet, so SmartScreen may ask once: **More info → Run anyway**.
3. A teal icon appears in the system tray; click it for the menu.

## Features

- **Screenshots** — interactive region selection with a frozen-frame dimmed overlay and
  rubber-band, window picker with highlight, single-monitor, and all-monitors capture.
- **Flyout** — a non-modal preview after each capture with quick actions: Annotate, Open, Open Folder,
  Copy, Copy Path, Delete.
- **Annotate** — draw red rectangles and red text boxes on a screenshot, then save an annotated
  copy or copy it to the clipboard.
- **Clipboard** — copy captures straight to the clipboard as an image.
- **Screen recording** — continuous Windows Graphics Capture → Media Foundation **H.264/MP4**,
  with constant-frame-rate pacing, pause/resume, and a recording toolbar.
- **Audio** — optional **system loopback** and **microphone** capture (WASAPI), mixed and muxed
  as **AAC** into the MP4, with sample-clock A/V sync. Settings has a **Microphone** list to
  pick which microphone is recorded (or follow the Windows default); a device that fails or is
  unplugged mid-recording leaves the rest of the recording intact and is reported afterwards.
- **GIF export** — record directly to an optimized animated GIF (downscaled, bounded by
  duration/FPS caps).
- **Global hotkeys** (rebindable, with conflict checks), a **Settings** dialog,
  **run-at-startup**, Per-Monitor-V2 **DPI awareness**, and single-instance behavior.
- **Robust by default** — a broken `config.toml` is set aside instead of blocking startup,
  files are written atomically and never overwritten, and every failure is shown to the user
  with its reason (details in `%APPDATA%RustShotlogs`).

## Default hotkeys

| Action                | Default shortcut            |
| --------------------- | --------------------------- |
| Region screenshot     | `PrintScreen`               |
| Window screenshot     | `Ctrl` + `PrintScreen`      |
| Monitor screenshot    | `Alt` + `PrintScreen`       |
| All-monitors capture  | `Shift` + `PrintScreen`     |
| Start region recording| `Ctrl` + `Shift` + `R`      |
| Stop recording        | `Ctrl` + `Shift` + `S`      |

All hotkeys are configurable in Settings. If another app already owns a combination (e.g. the
Windows 11 Snipping Tool holding `PrintScreen`), RustShot keeps running, says so in the tray
tooltip, and the tray menu still offers every action.

## Requirements

- Windows 10 1903+ / Windows 11 (x64). Uses Windows Graphics Capture, Direct3D 11, Media
  Foundation, and WASAPI.
- To **build**: Rust 1.90+ (the repo pins 1.98.1 in `rust-toolchain.toml`; rustup picks it up)
  with the `x86_64-pc-windows-msvc` toolchain, and the MSVC build tools
  (Visual Studio 2022 + Windows SDK).

## Build & run

```sh
# Debug build + run the tray app
cargo run -p app --bin rustshot

# Release build
cargo build --release --locked
# -> target/release/rustshot.exe (tray app) and target/release/rustshot-dev.exe (diagnostics)
```

### Quality gate

Every change must pass the same checks CI runs (CI also runs `cargo deny check`, builds the
portable zip, and checks the workspace still compiles with Rust 1.90):

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --workspace --locked
cargo test --workspace --locked
cargo deny check
```

Use `--locked` and commit `Cargo.lock` with any dependency or version change: CI builds with
`--locked` and fails on a stale lockfile.

Supply-chain policy (licenses, advisories, sources) is enforced with
[`cargo-deny`](https://github.com/EmbarkStudios/cargo-deny):

```sh
cargo install cargo-deny   # once
cargo deny check
```

## Developer / diagnostic CLI

The build produces two programs from one library (`crates/app`): `rustshot.exe`, the tray app
(GUI subsystem, no console), and `rustshot-dev.exe`, a console program with headless
diagnostics that never start the tray. Only `rustshot.exe` ships in the portable zip. Examples:

```sh
rustshot-dev config                       # print the loaded (repaired) config + its path
rustshot-dev paths                        # print all resolved RustShot paths
rustshot-dev list-monitors                # enumerate monitors
rustshot-dev list-windows                 # enumerate capturable windows
rustshot-dev list-audio-inputs            # microphones usable as microphone_device
rustshot-dev grab-monitor 0 shot.png      # full-monitor PNG
rustshot-dev grab-all desktop.png         # whole virtual desktop
rustshot-dev record-monitor 0 5 clip.mp4  # record monitor 0 for 5s to MP4
rustshot-dev record-gif 0 3 clip.gif      # record monitor 0 for 3s to GIF
rustshot-dev probe-mp4 clip.mp4           # decode back and report size/sample count
rustshot-dev settings-selftest            # show the Settings dialog briefly, then auto-close it
```

`rustshot-dev --help` lists every command.

## Packaging

- **Portable zip:** `scripts/build-portable.ps1` → `dist/RustShot-<version>-portable-x64.zip`.
- **Releases:** pushing an annotated `v<version>` tag runs `.github/workflows/release.yml`, which
  re-runs the gate and publishes the zip plus its `.sha256` as a GitHub release.
- **MSI installer:** per-user WiX v3 source in `wix/main.wxs`; not built or tested yet.

See [`docs/PACKAGING.md`](docs/PACKAGING.md) for details.

## Project layout

```
rustshot/
├─ crates/
│  ├─ capture-core/   # capture traits/types + Windows (WGC/D3D11) backend
│  ├─ recorder/       # Media Foundation H.264 encoder, WASAPI audio, muxing, GIF
│  ├─ imaging/        # crop, PNG/JPEG/GIF/ICO encode, thumbnailing, auto-naming
│  ├─ config/         # settings schema (serde), load/save, defaults, hotkey model
│  └─ app/            # lib `rustshot_app` + bins `rustshot` (tray) and `rustshot-dev` (CLI)
│     └─ src/         # ui/ (tray, overlay, flyout, annotate, recording toolbar, settings),
│                     # flows/ (capture, record), platform/ (DPI, GDI), dispatch, hotkeys, dev
├─ docs/              # MANUAL-GUIDE (en, vi), USER-GUIDE, PACKAGING, PLAN, DECISIONS,
│                     # PROGRESS, QA-CHECKLIST
├─ .github/           # CI and release workflows, shared toolchain setup action
├─ wix/               # WiX MSI source
└─ scripts/           # packaging scripts
```

## Documentation

- [Manual Guide](docs/MANUAL-GUIDE.md) · [Hướng dẫn sử dụng (Tiếng Việt)](docs/MANUAL-GUIDE.vi.md) — step by step, for users
- [User guide](docs/USER-GUIDE.md) — compact reference, including every `config.toml` key (shipped in the zip)
- [Packaging](docs/PACKAGING.md) — portable zip, releases, MSI
- [Development plan](docs/PLAN.md) · [Design decisions](docs/DECISIONS.md) · [Progress](docs/PROGRESS.md) · [QA checklist](docs/QA-CHECKLIST.md)

## License

[MIT](LICENSE)
