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

## M6 — Polish (settings UI, run-at-startup, manifest, USER-GUIDE)  ⬜ Next.

## M7 — Installer / packaging  ⬜ Not started.

---

### Verification log
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
