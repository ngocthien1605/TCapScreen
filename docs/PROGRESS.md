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

## M1 — Capture engine (stills)  🟡 in progress
See PLAN.md. Next: WGC/D3D11 backend, monitor/window enumeration, stride-correct `grab()`.

## M2–M7  ⬜ Not started.

---

### Verification log
- 2026-10-22: Toolchain smoke test — minimal cargo bin builds, links (MSVC auto-detected
  despite Git's `link.exe` on PATH), and runs. crates.io reachable.
- 2026-10-22: M0 gate green (build/clippy -D warnings/fmt/test: 49 tests pass). `dev paths`
  and `dev config` produce correct output. Live run: tray starts + enters message loop,
  first-run config written to `%APPDATA%\RustShot`, logs written, single-instance enforced
  (2nd instance exits), process stops cleanly.
