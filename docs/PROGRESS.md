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

## M3 — Recording (MP4)  ⬜ Next.

## M4–M7  ⬜ Not started.

---

### Verification log
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
