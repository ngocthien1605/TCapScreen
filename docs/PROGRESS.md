# PROGRESS — RustShot

Status legend: ✅ done & verified · 🟡 in progress · ⬜ not started · 🔬 manual-QA only (GPU/UI)

## M0 — Scaffold  ✅ (done & verified)

| Item | Status | How verified |
|---|---|---|
| Cargo workspace + 5 crates + toolchain pin | ✅ | `cargo build --workspace` green |
| Centralised dependency pins, Cargo.lock | ✅ | committed `Cargo.lock` |
| `config`: schema, defaults, load/save, hotkey model | ✅ | 20 unit tests (serde round-trip, partial/unknown keys, save→load identity, sanitize, hotkey parse/normalize/conflict) |
| `capture-core`: types + rect/DPI math | ✅ | 14 unit tests (intersect/clamp/union, from_points, DPI scale, stride-aware row) |
| `imaging`: crop, PNG/JPEG encode, auto-naming | ✅ | 12 unit tests (BGRA→RGBA + stride, crop bounds, png/jpeg round-trip, naming, unique_path) |
| `recorder`: typed stub | ✅ | 1 unit test (start returns NotImplemented, no panic) |
| `app`: logging, single-instance, tray stub menu, dev CLI | ✅ | build + 2 icon tests; **live run** (see log) |
| CI (fmt+clippy+build+test on windows-latest) | ✅ | `.github/workflows/ci.yml` |

**Gate (all green):** `cargo build` ✅ · `cargo clippy --workspace --all-targets -- -D warnings` ✅
· `cargo fmt --all -- --check` ✅ · `cargo test --workspace` ✅ **49 passed, 0 failed**.

**Done-when (M0):**
- App runs to tray and quits cleanly — ✅ **verified live**: process launches, enters the
  Win32 message loop ("RustShot is running in the tray"), stops cleanly.
- Settings round-trip to disk — ✅ unit-tested + first-run wrote `config.toml` to `%APPDATA%`.
- Single-instance — ✅ **verified live**: a second launch detected the mutex and exited.
- 🔬 Remaining manual QA: visually confirm the tray icon renders and menu items click
  (inherently interactive; see QA-CHECKLIST.md). Capture actions are intentional no-ops (M1/M2).

## M1 — Capture engine (stills)  ✅ (done & verified; multi-DPI caveat below)

| Item | Status | How verified |
|---|---|---|
| WGC/D3D11 device + staging-copy → `Frame` (stride = RowPitch) | ✅ | live capture; stride unit-tested |
| Monitor enumeration (bounds, DPI scale, primary) | ✅ | `dev list-monitors` live |
| Window enumeration (filter cloaked/tool/untitled) | ✅ | builds; exercised in M2 window-pick |
| `grab()` Monitor | ✅ | `dev grab-monitor` → valid PNG (scripted check) |
| `grab()` Region (crop) / Window / VirtualDesktop (composite) | ✅ (code) | `Frame::crop`/`blit_into` unit-tested; UI exercise in M2 |
| `imaging::is_near_black` DRM warning | ✅ | unit test |
| Per-Monitor-V2 DPI awareness at startup | ✅ | code; effect visible in correct bounds |

**Gate (all green):** build ✅ · clippy -D warnings ✅ · fmt ✅ · **53 unit tests pass**.

**Done-when (M1):** a dev command saves a correct full-monitor PNG.
- ✅ **verified live**: `dev grab-monitor 0 dev-out/m0.png` → 1920×1080 PNG; scripted
  validation: correct dimensions, luminance stddev 75.6, 255 distinct sampled colours
  (real content, not black/uniform). BGRA→RGBA channel order unit-tested.
- 🔬 **multi-monitor + mixed-DPI not hardware-verifiable here**: this dev box has a single
  1920×1080 display at 100%. The rect/DPI/union/crop/composite math is fully **unit-tested**;
  the live WGC path is confirmed on the one available display. Running on a mixed-DPI
  multi-monitor rig remains a manual QA item (QA-CHECKLIST M1).

## M2 — Screenshots complete  ✅ (automatable paths verified; interactive UI = manual QA)

| Item | Status | How verified |
|---|---|---|
| Action bus (hidden dispatch window; menu/hotkey/timer/flyout post `Action`s) | ✅ | live run; tray + loop |
| Monitor-under-cursor + all-monitors (virtual desktop) grab | ✅ | `dev grab-all` → 1920×1080 PNG (scripted: stddev 83.3) |
| Region grab (freeze → crop) | ✅ | `dev grab-region 100 100 640 480` → 640×480 (stddev 33.7) |
| Window grab (clean recapture) | ✅ | `dev grab-window Brave` → 1920×1032 JPEG (stddev 71.3) |
| Window enumeration (filter cloaked/tool/untitled/minimized-offscreen) | ✅ | `dev list-windows` live |
| Clipboard image copy (arboard `image-data`) | ✅ | `dev clipboard-image` → PS `Clipboard.GetImage()` returns 640×480 |
| Auto-name + unique_path + save (PNG/JPEG) | ✅ | files written with `RustShot_…` names; unit-tested |
| Global hotkeys (map + register + route → dispatch) | ✅ | live: Ctrl+Shift+R/S registered; PrintScreen combos claimed by OS → warn, non-fatal |
| Delay timer (worker thread → `ExecuteDelayedShot`) | ✅ (code) | no-sleep-on-UI design; fires capture after N s |
| Pure selection state machine (drag/click/esc/enter, window pick, label placement) | ✅ | 10 unit tests |
| Hotkey→Code mapping table | ✅ | 6 unit tests |
| Region overlay window (frozen backdrop, dim, rubber-band, highlight, W×H label) | 🔬 | compiles + clippy-clean; **visual → manual QA** |
| Post-capture flyout (thumbnail + 5 buttons + auto-dismiss) | 🔬 | compiles + clippy-clean; **visual → manual QA** |

**Gate (all green):** build ✅ · clippy -D warnings ✅ · fmt ✅ · **69 unit tests pass** (was 53).

**Done-when (M2):**
- A screenshot can be taken and saved+copied — ✅ **verified live** for monitor/window/region/
  all-monitors via the `dev` commands, with scripted validation of dimensions + real content
  and a confirmed clipboard round-trip.
- 🔬 **Interactive overlay + flyout are visual/UI and not scriptable here**: the Win32 code
  builds and is clippy-clean, and the *pure* interaction logic (what to select, where to place
  the label, which window wins) is unit-tested. Exercising the dimmed overlay, rubber-band,
  window highlight, and the five flyout buttons is a manual-QA item (QA-CHECKLIST M2).
- 🔬 Mixed-DPI multi-monitor behaviour still needs a suitable rig (single 100% display here).

## M3 — Recording (MP4)  ✅ (engine verified end-to-end; toolbar UI = manual QA)

| Item | Status | How verified |
|---|---|---|
| Continuous WGC stream (`CaptureStream`): multi-buffer pool, FrameArrived→channel, Closed event | ✅ | live 4–5 s recordings |
| Media Foundation `IMFSinkWriter` H.264/MP4, RGB32 in (auto NV12 convert) | ✅ | `probe-mp4` decodes it |
| CFR re-emission driven by the monotonic clock (static screen stays smooth) | ✅ | 5 s → 140 samples, last_ts 4.63 s ≈ 30 fps, correct speed |
| Even-dimension rounding (H.264 rejects odd) | ✅ | 1 unit test + odd window sizes |
| Top-down orientation (positive input stride) | ✅ | frame-0 vs live grab: top bright in both (not flipped) |
| Finalize-on-drop / on-stop (writes `moov`) | ✅ | files play/decode; Drop calls stop() |
| Dropped/coalesced-frame logging | ✅ | log line (captured vs emitted) |
| Pause/resume (clock stops; output excludes paused time) | ✅ | `record-pause-test 0 6 2 4` → 3.6 s output (≈ 5.6 s baseline − 2 s paused) |
| Region/window/monitor targets | ✅ | monitor via `dev record-monitor`; region/window via overlay target-select |
| Source-closed → stop + finalize | ✅ (code) | WGC `Closed` event path; QA item for the live case |
| Countdown + compact always-on-top toolbar (elapsed + Pause/Stop) | 🔬 | compiles + clippy-clean; **visual → manual QA** |
| `dev record-monitor` / `dev probe-mp4` functional tests | ✅ | see log |

**Gate (all green):** build ✅ · clippy -D warnings ✅ · fmt ✅ · **70 unit tests pass**.

**Done-when (M3):** a monitor recording produces a correct, playable MP4.
- ✅ **verified live**: `dev record-monitor 0 5 rec.mp4` → 2 MB MP4; `dev probe-mp4` (MF
  `IMFSourceReader`, the same decode stack as Windows Media Player) decoded **140 samples at
  1920×1080**, duration 4.63 s (correct 30 fps speed after the pacing fix), and frame 0 matched
  a live grab in size, content (stddev ≈ 81), and orientation (not flipped).
- 🔬 **Actual playback in VLC/WMP, and the interactive countdown/toolbar/pause buttons, are
  visual and remain manual QA** (QA-CHECKLIST M3). SourceReader decoding is the headless proxy.

## M4 — Audio  ✅ (system path verified with a synthetic tone; mic = manual QA)

| Item | Status | How verified |
|---|---|---|
| WASAPI system (render-endpoint loopback) capture | ✅ | `record-audio-test` → probe RMS 0.21, dominant ≈1020 Hz |
| Silence keep-alive render client (loopback doesn't starve) | ✅ | same path (doubles as tone source in the test) |
| WASAPI microphone (default capture endpoint) | ✅ (code) | same code path minus loopback; **needs a mic signal → 🔬** |
| Mix sources → 48 kHz stereo, linear resample, soft-clamp | ✅ | mixed path builds; single-source verified |
| AAC stream added to the MP4 (MF `IMFSinkWriter`, PCM16 in) | ✅ | `probe-mp4` decodes an audio stream |
| Sample-count audio clock (can't drift vs the video index clock) | ✅ | audio 4.61 s vs video 4.57 s (~42 ms) |
| A/V start alignment (pre-roll discarded at loop start) | ✅ | skew dropped from ~0.43 s to ~0.04 s |
| Pause drops audio (stays aligned with paused video) | ✅ (code) | same pause gate as video |
| Graceful "no audio device" → video-only | ✅ (code) | setup failure logs + records video-only |
| Audio probe (RMS + zero-crossing dominant Hz) | ✅ | `probe-mp4` prints it |

**Gate (all green):** build ✅ · clippy -D warnings ✅ · fmt ✅ · **70 unit tests pass**.

**Done-when (M4):** audio muxes correctly and stays in sync.
- ✅ **verified live**: `dev record-audio-test 0 5 audio.mp4` renders a 1 kHz tone into the
  render keep-alive, captures it via loopback, encodes AAC, muxes, and `probe-mp4` decodes the
  **audio stream at 48 kHz/2ch, RMS 0.21 (not silence), dominant ≈1020 Hz** (≈1 kHz within the
  zero-crossing estimate's tolerance), with **audio duration within ~42 ms of the video**.
- 🔬 **Microphone capture and mic+system mixing need a live mic signal**, and A/V lip-sync by
  ear is inherently manual (QA-CHECKLIST M4). The system path proves capture→mix→AAC→mux→decode.

## M5 — GIF  ✅ (verified: capture → downscale → encode → decode)

| Item | Status | How verified |
|---|---|---|
| GIF capture via WGC stream, bounded by fps × max_seconds | ✅ | `record-gif 0 3` → 14 frames at 15 fps |
| Downscale to `gif_max_width` (longer edge, aspect preserved, no upscale) | ✅ | 1920×1080 → **640×360** |
| Animated looping GIF encode (`image` crate) | ✅ | `imaging::probe_gif` decoded 14 frames |
| Length cap (`gif_max_seconds`) and fps clamp (≤50) enforced | ✅ | frame-count bound in the loop; clamp in code |
| Shares the recorder handle (pause/stop) via a `Backend` enum | ✅ | builds; same handle API |
| Tray "Record GIF (region/window)" wired | ✅ (code) | tray item → `RecordGif` → overlay target select |

**Gate (all green):** build ✅ · clippy -D warnings ✅ · fmt ✅ · **70 unit tests pass**.

**Done-when (M5):** a short recording exports an optimised, browser-openable GIF.
- ✅ **verified live**: `dev record-gif 0 3 clip.gif` → **640×360, 14 frames**, re-decoded by an
  independent `GifDecoder` (valid animated GIF). Caps are enforced by construction.
- 🔬 Visual quality / palette optimisation and opening in a browser are a quick manual check
  (QA-CHECKLIST M5).

## M6 — Polish  ✅ (settings dialog UI = manual QA; rest verified)

| Item | Status | How verified |
|---|---|---|
| `windows_subsystem = "windows"` (no console) + attach-parent-console for dev | ✅ | `dev` still prints under the GUI subsystem (inherited-handle guard) |
| Run-at-startup (HKCU Run, re-writes current exe path) | ✅ | enable → key = quoted exe path; disable → removed (verified in registry) |
| App manifest: Per-Monitor-V2 DPI + asInvoker, embedded via build.rs/rc.exe | ✅ | builds with embedded resources; DPI still set programmatically too |
| Exe icon embedded (generated from the programmatic tray icon) | ✅ | `dev make-icon` + resource embed |
| Settings dialog (DLGTEMPLATE: checkboxes, quality/fps edits, folder+Browse) | 🔬 | pure `DlgBuilder` unit-tested (2 tests); dialog is visual → manual QA |
| Apply-on-OK: save config, update output folder, re-register hotkeys, startup | ✅ (code) | hotkeys moved into `AppState`; re-register drops old then registers |
| USER-GUIDE.md | ✅ | written (hotkeys, flyout, recording, config keys, troubleshooting) |

**Gate (all green):** build ✅ · clippy -D warnings ✅ · fmt ✅ · unit tests pass.

**Done-when (M6):**
- Settings persist and take effect — ✅ the apply path saves config and live-updates folder +
  hotkeys + startup; the dialog **template builder is unit-tested** (header/layout/item count).
  Driving the actual dialog (clicking controls) is visual → manual QA (QA-CHECKLIST M6).
- Run-at-startup — ✅ **verified** by inspecting the registry before/after.
- No console window; DPI manifest present — ✅ (subsystem + embedded manifest).

## M7 — Installer / packaging  ✅ (portable + release verified; MSI authored, builds on a VM)

| Item | Status | How verified |
|---|---|---|
| Release build (`lto=thin`, `strip`, no panic=abort) | ✅ | `cargo build --release` green |
| Portable zip script | ✅ | `scripts/build-portable.ps1` → `dist/RustShot-<ver>-portable-x64.zip` built & listed |
| WiX MSI source (`wix/main.wxs`): Program Files install, Start-menu shortcut, ARP entry, major-upgrade | ✅ (authored) | WiX not installed here → building the MSI is a release step (🔬) |
| `cargo-deny` policy (`deny.toml`): advisories, license allow-list, source/version bans | ✅ | `cargo deny check` → advisories/bans/licenses/sources all **ok** |
| MIT license + LICENSE file | ✅ | committed |
| `docs/PACKAGING.md` | ✅ | portable + MSI + cargo-deny instructions |

**Gate (all green):** build ✅ · clippy -D warnings ✅ · fmt ✅ · 75 unit tests.

**Done-when (M7):** a portable build and an installer exist.
- ✅ **portable zip built and verified** (contains the release exe + guide + license).
- 🔬 **The MSI and `cargo deny check` need their tools** (WiX, cargo-deny), which aren't present
  in this environment; the sources are authored and documented. Installing on a clean Windows 11
  VM (install → Start-menu launch → uninstall) is the release-time QA item (QA-CHECKLIST M7).

---

### Verification log
- 2026-10-23: Hardware checks, first pass (tester on the 0.1.6 tray build, results on the
  checks page, read back with the log). Pass: Settings taskbar button, two monitors at
  different scales, GIF length cap with notice, mic+system recording by ear, 102 s A/V sync by
  ear. Found: every recording used the Windows default microphone (Realtek, later the built-in
  Intel array once AirPods connected), never the earphone/AirPods mic the tester meant; that is
  what the (unreleased) Settings microphone picker fixes. So the Bluetooth-mic and mic-unplug
  checks did not test the microphone: the unplug dropped the AirPods' output and the
  system-audio lane stopped (`0x88890004`), file saved, dialog correct, mic carried on. Not run
  yet: read-back throttle, tray tooltip, run at startup, display unplug, GPU reset.
- 2026-10-23: Microphone picker in Settings (group 3, decision 1): a drop-down of "System
  default (current default)" plus the connected microphones, built with the recorder's own
  matching rule (`recorder::find_input`), storing the device name in `microphone_device`; a
  configured but disconnected device is kept as "not connected"; greyed out while "Record
  microphone" is off. Template +2 controls (37), 3 new unit tests (175 total);
  `settings-selftest` renders it with 2 entries. Not driven by hand: the tray was in use for the
  hardware checks.
- 2026-10-23: **Release 0.1.6** (tag `v0.1.6`): the app crate refactor (review item 7, two
  binaries: `rustshot.exe` ships, `rustshot-dev.exe` is for development) and the small fixes
  (config bounds and `schema_version`, overlay focus for delayed shots, Settings taskbar
  button, tray tooltip refresh, disk-full reporting, test hygiene).
- 2026-10-23: Review item (7) — app crate refactor: library + `rustshot` (tray) and
  `rustshot-dev` (console diagnostics) binaries, the parent-console hack removed; dev commands
  return `Result` with shared steps (960 → 616 lines, outputs unchanged); `settings_ui` split
  into `ui/settings/{template,dialog,capture}`; `platform/` (DPI, owned GDI handles reused by
  the editor); `ui/` and `flows/` directories. App crate 7970 → ~7650 lines; tray exe 4.1 →
  3.3 MB. Re-verified live: screenshot → flyout → editor (draw, prompt, save with the red
  rectangle in the file), Settings + Browse, region recording start/stop.
- 2026-10-23: Small fixes after (7): `schema_version` in config (raised quietly, never
  lowered), sanitize bounds for GIF width/length, screenshot delay and countdown; UTC fallback
  for file names is logged; MF/IO disk-full errors map to `RecorderError::DiskFull`; logging
  falls back to stderr when there is no log directory; tray tooltip refreshes when Settings
  re-registers hotkeys; Settings gets a taskbar button and opens in front; the overlay takes
  keyboard focus even without recent input (delayed shots); tests use self-cleaning temp dirs
  and the path tests can fail; unused Windows features dropped; CI on checkout@v5 /
  upload-artifact@v6 (Node 24). Live: overlay foreground + real Esc, Settings taskbar/front.
- 2026-10-23: **Release 0.1.5** (tag `v0.1.5`), the first published by the release workflow:
  audio fixes (review item 4), pinned toolchain / cargo-deny / MSRV / release CI (item 5), and
  the medium findings (item 6: DPI scaling, themed controls, GIF timing, atomic saves, strict
  shortcut names, bounded logs, safer editor/overlay window state).
  The first tag push failed the release job at `cargo-deny`: `clipboard-win 5.4.1` had been
  yanked since the last green CI run. Updated it to 5.4.3 and moved the unpublished tag to the
  fixed commit; the workflow then published the release.
- 2026-10-23: Review item (6) — the medium findings. Overlay copy bounded by `Frame::row` and
  GDI guards; editor state ownership via a thread-local adopted in WM_NCCREATE (no double free),
  no `&mut` state across message boxes / DestroyWindow; Browse picker owned by Settings; flyout
  waits while hovered; strict shortcut key names (W3C list + aliases, rejected otherwise, test
  ties it to registration); logs keep 14 days, panic.log rolls at 1 MB; screenshots/annotated
  copies claim names with create_new and write atomically (rename retried for OneDrive); release
  build requires embedded resources; shared `FrameClock` with exact frame timestamps; GIF frames
  timed by slots (playback = recorded time) and a notice at the length cap; GPU read-backs capped
  at the output fps with a held frame + `pump()`, reused staging texture; stream start no longer
  leaks its handler on error; DPI-scaled flyout/toolbar/editor toolbar with the system font and
  per-monitor placement; Common Controls v6; per-user MSI source (unbuilt). Live (debug build):
  overlay, flyout hover, editor close/prompt/save, Browse modality, 4 rapid screenshots into
  OneDrive, MP4 + GIF timing, themed Settings, scaled flyout/toolbar/editor at 125 %.
- 2026-10-23: Review item (5) — CI and releases. `rust-toolchain.toml` pinned to `1.98.1`
  (was `stable`); CI installs the toolchain named there (composite action
  `.github/actions/setup-rust`), adds `cargo deny check` (cargo-deny 0.20.2, same as local),
  `--locked` builds, the portable zip as a workflow artifact, and an `msrv` job checking with
  `rust-version` 1.90 (passes locally with 1.90.0). New `release.yml`: on a `v*` tag (or run by
  hand for an existing tag) re-runs the gate, checks tag = version, builds the zip + `.sha256`,
  and publishes the GitHub release with the tag message as notes.
- 2026-10-23: Review item (4) — audio. Each source is set up on its own (a failing mic no longer
  discards system audio); `microphone_device` is honoured (id / name / partial name, falling
  back to the default with a note); PCM 8/24/32-bit and float64 decode (were silent), channel
  mask downmix (centre kept), boundary-continuous resampler, `GetMixFormat` freed; device setup
  overlaps the first-frame wait with a 10 s bound and detaches on timeout; the mixer keeps a
  stalled source aligned (late audio for a padded span is dropped), a dead device becomes
  silence; the encoder holds the audio sample clock to the video clock (silence fill / trim
  beyond 200 ms). Problems become recording notes, shown in one dialog after the file is saved.
  New `dev list-audio-inputs`, `dev audio-probe` (in memory only). Live: mic, mic + system,
  bad and partial `--mic-device` via `audio-probe`; `record-audio-test 0 10` → audio 9.11 s /
  video 9.07 s, 1 kHz. Gate green, **151 tests**.
- 2026-10-23: **Release 0.1.4** (tag `v0.1.4`): Annotate editor (red rectangle / text, crosshair
  cursor, Shift/Alt+Enter new line), review items (1)–(3) (record-flow busy gate, Quit
  teardown, panic containment; config recovery + user-visible error dialogs; recordings end
  cleanly when the source window closes or the GPU device is lost). Portable zip
  `dist/RustShot-0.1.4-portable-x64.zip` built from the tagged commit.
- 2026-10-23: Review item (3) — capture-stream failure handling. `StreamEvent::Error` +
  bounded failure counter in the WGC callback, `check_health` poll (device removed-reason +
  `IsWindow`; WGC's `Closed` event did not fire for a killed window), frame pool `Recreate` on
  source resize, `pack_bgra` zero-fill (MF buffers are uninitialized), `RecorderError::EndedEarly
  { path, reason }`, `RecorderHandle::is_finished` + `Action::RecordPoll` from the toolbar tick
  so a self-ended recording is collected and reported ("The recording ended early … saved at
  …"). New `dev record-window`; `dev` commands now log to stderr. Live: window killed at 5 s →
  ended early in 250 ms, 121-sample playable MP4; resize → two pool recreates, file OK; tray
  app: toolbar gone in 0.7 s without Stop, notice shown, 3 s MP4 probes OK. Gate green, **137
  tests**.
- 2026-10-23: Review item (2) — config recovery + error surfacing. `Settings::load_or_recover`
  quarantines unparseable config (`config.toml.broken-<n>`), lenient hotkey deserialize
  (bad string → unbound), `sanitize()` resets a relative output folder, Settings OK validates
  the folder (absolute + creatable), startup/apply fall back to the default folder when the
  configured one cannot be created. New `notify` module (message box + log) now used for
  capture/recording/clipboard/delete/settings/hotkey failures; the panic dialog goes through
  it too. Hotkeys Windows refused are listed in the tray tooltip at startup and in a dialog on
  Settings OK. Live on the release build: garbage config → dialog + `.broken-` file + fresh
  defaults; relative folder → reset silently; `Z:\nope\shots` → "not usable" dialog with the
  `Pictures\RustShot` fallback. Gate green: fmt/clippy/deny, **129 tests**.
- 2026-10-22: M2 hardening (post-review). Fixed: (1) flyout/OpenFolder shell+clipboard work now
  runs *outside* the `AppState` borrow (ShellExecuteW can pump messages and re-enter dispatch —
  would have been a `BorrowMutError` panic in a wndproc); (2) overlay nested loop now destroys
  its window and re-posts `WM_QUIT` if a quit/err interrupts it (no dangling `GWLP_USERDATA`);
  (3) `dev` commands now init COM **STA** to mirror the tray app. Checks: WGC capture works
  from STA (`grab-all` OK); a non-maximized window captured at exactly its frame-bounds size
  (1235×628 — no resize-border inflation, so no extra crop needed) **and** exercised the
  stride-padding path on real hardware (stride 4992 ≠ width×4 4940). Confirmed the four
  PrintScreen combos are held by **Screenpresso** (PID 1108), which is running on this box —
  the graceful degradation worked as designed.
- 2026-10-22: M2 gate green (build/clippy -D warnings/fmt/test: **69 tests pass**). Live:
  `dev list-windows` (5 windows), `grab-all` (1920×1080), `grab-region 100 100 640 480`
  (640×480), `grab-window Brave` (1920×1032 JPEG); scripted System.Drawing check confirmed
  dimensions + real content (lum-stddev 83.3/33.7/71.3). `dev clipboard-image` → PowerShell
  `Clipboard.GetImage()` returned a 640×480 bitmap (round-trip OK). Tray app boots, installs
  the dispatch window, Ctrl+Shift+R/S hotkeys registered; PrintScreen combos already held by
  the OS/Snipping Tool → logged as warnings, app continued (graceful degradation).
- 2026-10-22: Toolchain smoke test — minimal cargo bin builds, links (MSVC auto-detected
  despite Git's `link.exe` on PATH), and runs. crates.io reachable.
- 2026-10-22: M0 gate green (build/clippy -D warnings/fmt/test: 49 tests pass). `dev paths`
  and `dev config` produce correct output. Live run: tray starts + enters message loop,
  first-run config written to `%APPDATA%\RustShot`, logs written, single-instance enforced
  (2nd instance exits), process stops cleanly.
