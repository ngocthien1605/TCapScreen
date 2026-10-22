# PLAN — RustShot

A lightweight, tray-resident Windows app that does exactly two things: **take screenshots**
and **record the screen**. Clean-room build inspired by Screenpresso 2.3.10.0 (capabilities,
not code/assets). See `../RustShot-MASTER-PROMPT.md` for the authoritative spec.

## Phase breakdown (each phase ends green: build + clippy -D warnings + fmt --check + test)

- **M0 — Scaffold.** Cargo workspace, toolchain, logging, config load/save, tray icon with a
  stub menu, single-instance, dev CLI skeleton, CI. *Done when:* app runs to tray and quits
  cleanly; settings round-trip to disk (unit-tested + `--dev config` command).
- **M1 — Capture engine (stills).** WGC/D3D11 backend: enumerate monitors/windows, `grab()` a
  full monitor frame to CPU (stride-correct), encode PNG. *Done when:* `--dev grab-monitor`
  saves a correct full-monitor PNG on a multi-monitor, mixed-DPI setup.
- **M2 — Screenshots complete.** Region overlay, window pick, all-monitors, timer, cursor
  toggle, clipboard, auto-naming, flyout, hotkeys, tray wiring. *Done when:* all §6.1 modes
  work end-to-end from hotkeys with the flyout.
- **M3 — Recording (video-only MP4).** WGC continuous stream → ring buffer → MF H.264 → MP4;
  countdown + recording toolbar + Stop/Pause; FPS/bitrate/cursor; dropped-frame accounting.
  *Done when:* a 60 s 1080p30 recording plays back correctly with stable timing.
- **M4 — Audio.** Mic (`cpal`) + system loopback (WASAPI), device selection, mixing, A/V-sync
  mux. *Done when:* a mic+system recording is in sync (±1 frame over 3 min).
- **M5 — GIF export.** Frames → palette-quantised optimised GIF with caps + warnings.
- **M6 — Product polish.** Settings UI, run-at-startup, error dialogs, history list, original
  branding, USER-GUIDE.md.
- **M7 — Package & ship.** MSI/NSIS installer + portable zip, cargo-deny pass, signing hook.

## Workspace layout

```
rustshot/
├─ Cargo.toml                # workspace + centralised pins
├─ rust-toolchain.toml
├─ crates/
│  ├─ capture-core/          # platform-agnostic traits + types; Windows (WGC) backend
│  ├─ recorder/              # MF H.264 encoder + audio capture/mix + muxing + GIF
│  ├─ imaging/               # crop, PNG/JPEG encode, clipboard, thumbnailing, auto-naming
│  ├─ config/                # Settings schema (serde), load/save, defaults, hotkey model
│  └─ app/                   # BIN: tray, hotkeys, overlay, flyout, toolbar, settings, CLI
└─ docs/                     # PLAN, DECISIONS, PROGRESS, QA-CHECKLIST, USER-GUIDE
```

## Testing strategy

- **Unit-test the pure logic:** rect/DPI math, crop, auto-naming, config serde round-trip,
  hotkey parsing, GIF palette, ring-buffer back-pressure. These run in CI headlessly.
- **GPU/display paths** (WGC grab, MF encode, overlay, tray message loop) cannot run on a
  headless CI runner. They are exercised by a **dev CLI** (`rustshot --dev …`) runnable on a
  real desktop, and verified against the manual **QA-CHECKLIST.md**. We do not fake green:
  anything not auto-tested is labelled as manually-verified or not-yet-done in PROGRESS.md.

## Open questions (resolved pragmatically; logged in DECISIONS.md)

1. `windows-capture` (2.x) vs raw WGC+MF via `windows` — evaluate in M1/M3; start with the
   higher-level crate, keep a thin wrapper so we can drop to raw if its API/maintenance gaps.
2. Overlay tech: layered Win32 window via `windows` vs `winit`+`softbuffer`. `winit` is
   pre-release (0.31 beta); decide in M2, leaning to a thin Win32 layered window for precision.
3. UI toolkit for settings/flyout/toolbar: `egui`/`eframe`. Confirm it coexists with a Win32
   overlay and the `tray-icon`/`global-hotkey` event loops.
