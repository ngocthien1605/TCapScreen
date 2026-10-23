# PLAN — RustShot

A lightweight, tray-resident Windows app that does exactly two things: **take screenshots**
and **record the screen**. Clean-room build inspired by Screenpresso 2.3.10.0 (capabilities,
not code/assets). See `../RustShot-MASTER-PROMPT.md` for the authoritative spec.

## Status (2026-10-23, release 0.1.7)

All phases M0–M7 are done; [PROGRESS.md](PROGRESS.md) has the per-phase evidence and the
verification log. After M7 came a review pass (items 1–7: busy gate and teardown, config
recovery and error dialogs, capture-stream failures, audio, CI and releases, the medium
findings, the app crate split) and the Settings microphone picker. The latest release is 0.1.7,
published on GitHub with the earlier ones. What is still open:

- **Manual hardware checks** that need a person or devices (Bluetooth microphone through the
  picker, unplugging devices, GPU reset, run at startup, read-back throttle): tracked in
  [QA-CHECKLIST.md](QA-CHECKLIST.md) and on the hardware checks page it links to.
- **Not started, by choice:** code signing, a built and tested MSI, a capture history list
  (M6), and the 4K memory / D3D thread-safety follow-ups. Each waits for a concrete need.

The phase list below is the original plan, kept as written; where the build departed from it
(raw WASAPI instead of `cpal`, Win32 dialogs instead of egui), see the resolved questions at
the end and DECISIONS.md.

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
│  └─ app/                   # lib + bins rustshot (tray) and rustshot-dev (diagnostics)
└─ docs/                     # PLAN, DECISIONS, PROGRESS, QA-CHECKLIST, USER-GUIDE,
                             # MANUAL-GUIDE (en, vi), PACKAGING
```

## Testing strategy

- **Unit-test the pure logic:** rect/DPI math, crop, auto-naming, config serde round-trip,
  hotkey parsing, GIF palette, ring-buffer back-pressure. These run in CI headlessly.
- **GPU/display paths** (WGC grab, MF encode, overlay, tray message loop) cannot run on a
  headless CI runner. They are exercised by a **dev CLI** (`rustshot-dev …`) runnable on a
  real desktop, and verified against the manual **QA-CHECKLIST.md**. We do not fake green:
  anything not auto-tested is labelled as manually-verified or not-yet-done in PROGRESS.md.

## Open questions — resolved (details in DECISIONS.md)

1. `windows-capture` (2.x) vs raw WGC+MF via `windows` → **raw** WGC and `IMFSinkWriter`
   through the `windows` crate: control over stride, DPI and the audio mux (M1, M3).
2. Overlay tech → a **raw Win32** window over the virtual desktop, no `winit` (M2).
3. UI toolkit for settings/flyout/toolbar → **plain Win32** (an in-memory `DLGTEMPLATE` for
   Settings, small tool windows for the flyout and toolbar), not egui, which wanted to own the
   main-thread event loop (M6).
4. Audio (planned as `cpal` for the microphone) → **raw WASAPI** for both microphone and
   system loopback, since `cpal` cannot do loopback (M4).
