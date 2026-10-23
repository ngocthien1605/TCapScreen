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
- **No `panic = "abort"`** in release, and panics are actively contained (`app/src/panic_guard.rs`):
  every Win32 callback body (window/dialog procedures, the hotkey handler, the WGC frame
  callback) runs under `catch_unwind`, because a panic unwinding across an `extern "system"`
  boundary aborts the process — for a tray app that means the icon just vanishes. A caught panic
  is logged, shown once in a dialog, and the callback returns a neutral value. A process-wide
  hook also writes every panic synchronously to `logs/panic.log`, since the normal file logger
  is asynchronous and may never flush before an abort. Release keeps line tables in the `.pdb`
  (`debug = "line-tables-only"`) so those locations are symbolizable.
- **Failures are surfaced, not just logged** (`app/src/notify.rs`). Any failure the user
  triggered or would otherwise never learn about — a screenshot or recording that did not
  happen, a clipboard copy or delete that failed, settings that did not save, hotkeys Windows
  refused — goes through `notify::{error,warn,notice,fatal}` as well as `tracing`. The seam is
  a plain top-most message box today: `tray-icon` 0.26 has no balloon API, and a custom toast is
  more than the seam needs; the API makes no modality promise so a toast can replace it later
  without touching call sites. Rule for callers on the UI thread: notify *before* releasing the
  `busy` gate, because the box pumps messages and a hotkey pressed meanwhile would otherwise
  start a capture nested inside it. Exception: hotkeys another app already holds are reported at
  startup only in the tray tooltip + log (a permanent condition on that machine should not pop a
  dialog on every login); Settings → OK reports them in a dialog because the user just chose them.
- **A broken `config.toml` never blocks startup** (`Settings::load_or_recover`). A file that is
  not valid TOML is renamed to `config.toml.broken-<unix-secs>` (so hand-written hotkeys can be
  recovered), defaults take over, a fresh file is written, and a startup dialog names both
  files; an *unreadable* file (I/O error) is left alone and defaults apply for the session only.
  Within a valid file, a shortcut string that does not parse deserializes as *unbound* instead of
  rejecting the whole file (trade-off: a typo shows up as a dead key, not an error; the log
  names it). A *relative* output folder is reset to the default by `sanitize()` — it would
  resolve against `System32` when launched from the `Run` key — and the Settings dialog refuses
  one on OK; an absolute folder that cannot be created falls back to the default with a dialog.
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

## M2 — Screenshots complete

- **Overlay = raw Win32, a single window over the virtual-desktop bounding box, painting a
  *frozen* capture.** No winit/eframe (avoids a second event loop fighting the tray's
  `GetMessageW` pump; winit 0.31 is beta). No `WS_EX_LAYERED` transparency: we freeze the
  desktop first (one `grab(VirtualDesktop)`), dim it with a 1×1 AlphaBlend, and restore the
  selection/hover region undimmed. The shot is cropped from the frozen frame, so the overlay
  can never leak into it and there is no hide-then-recapture race.
- **Action bus (`dispatch.rs`).** `MenuEvent`/`GlobalHotKeyEvent` handlers must be
  `Fn + Send + Sync + 'static`, so they can't hold app state; they `PostMessage` a `repr(u32)`
  `Action` to a hidden `HWND_MESSAGE` window whose proc owns all state via a thread-local
  `AppState`. A `busy` flag drops a second trigger while an overlay is open. HWND isn't `Send`,
  so handlers carry it as `isize` and rebuild it on the UI thread (where they actually fire).
- **Pure interaction core (`overlay/state.rs`, `hotkeys.rs`).** The selection state machine and
  the hotkey→`Code` table are Win32-free and unit-tested; the `win.rs`/`register` layers are
  thin plumbing. This is what keeps M2 honest on a machine where the overlay itself can't be
  scripted.
- **Window pick shares the overlay.** Click-without-drag (< 4 px) selects the top-most window
  under the cursor (`EnumWindows` z-order, first containing match); result is a clean
  `grab(Window)` recapture, falling back to a frozen-crop of the window bounds on error.
- **Capture flow never holds the `AppState` borrow across the nested overlay loop.** It clones
  the few needed settings out, runs with a fresh (zero-sized) `WindowsBackend`, then re-enters
  only to stash the flyout and clear `busy` — so re-entrant actions during an open overlay are
  safe.
- **Delay/timer off the UI thread.** A worker thread sleeps then posts `ExecuteDelayedShot`;
  the UI pump never blocks (per-second countdown UI is deferred — it belongs with the recording
  toolbar in M4).
- **Main thread is COM STA** (`CoInitializeEx`) for shell verbs / future file dialogs; the
  capture backend's `RoInitialize(MTA)` then returns `RPC_E_CHANGED_MODE`, tolerated — WGC's
  free-threaded pool is apartment-agnostic.
- **Hotkey registration failures are warnings, not fatal.** On Windows 11 `PrintScreen` (and
  often its combos) are held by the Snipping Tool / other tools; we log and continue, and the
  tray menu always provides the same actions.
- **`arboard` needs its `image-data` feature re-enabled** on the `app` dependency (the workspace
  pins it `default-features = false`), or `set_image` wouldn't exist.
- **Flyout is Win32, non-modal, StretchDIBits thumbnail.** It stores a heap-stable `Box` so its
  window proc can reach it by pointer; buttons post `Flyout*` actions back through the dispatch
  window, keeping all mutation on the UI thread.

## M3 — Recording (MP4, video-only)

- **Raw `IMFSinkWriter`, not `windows-capture`.** The decisive constraint is M4: the muxer must
  accept an AAC audio stream beside the video. `IMFSinkWriter::AddStream` does that natively, so
  starting here avoids a rewrite when audio lands. Input type is `MFVideoFormat_RGB32` (== our
  BGRA) and the SinkWriter inserts the color converter to NV12 for the H.264 encoder.
- **Continuous capture lives in `capture-core` (`CaptureStream`)**, reusing the M1 D3D device
  and the extracted `wgc::frame_to_cpu` helper; `recorder` owns only the MF encode + pacing.
  The staging copy runs on the WGC pool threads behind a `Mutex<D3dDevice>` (the D3D immediate
  context is not thread-safe).
- **Stream failure model: poll the device, never a frame clock.** WGC delivers a frame only
  when the content changes, so "no frame for N seconds" is normal on an idle desktop and must
  not be treated as a stall. What the stream *does* report: the GPU→CPU copy failing repeatedly
  (a removed/reset D3D device never recovers) sends one `StreamEvent::Error` after a bounded run
  of failures (logged sparsely, not at refresh rate); the encoder loop additionally polls
  `CaptureStream::check_health` once a second, which asks the device for its removed-reason and,
  for window targets, `IsWindow` — WGC was observed **not** to raise `GraphicsCaptureItem.Closed`
  for a window killed mid-capture, so the poll is the path that actually ends such a recording.
  On any early end the sink is still finalized and the recorder returns
  `RecorderError::EndedEarly { path, reason }` so the user is told where the partial file is;
  the toolbar's 500 ms tick posts `RecordPoll` so the app collects a self-ended recording
  instead of counting over it. Source resizes recreate the frame pool at the new content size
  (the documented sample pattern, done inside `FrameArrived` after the frame is closed); the MP4
  sink keeps its initial dimensions, so a grown source is cropped and a shrunken one is padded
  with black (`pack_bgra` zero-fills — `MFCreateMemoryBuffer` memory is uninitialized).
- **CFR re-emission on the monotonic clock.** WGC sends nothing while the screen is static, so a
  write-on-arrival file would freeze/jump. A dedicated encoder thread keeps the latest frame and,
  each iteration, writes enough copies to make the emitted frame count equal
  `active_elapsed / frame_dur`. This both fixes static-screen smoothness and makes the output's
  duration track real time instead of drifting with `sleep` latency (an early sleep-per-frame
  version produced a 3 s file for a 4 s recording — ~33 % fast).
- **Pause = stop the clock.** Paused wall-time is subtracted from `active_elapsed`, so the index
  doesn't advance and the output simply omits the paused span; resume continues seamlessly.
- **Even dimensions + top-down stride.** H.264 rejects odd width/height, so dims are rounded down
  to even from the first captured frame. `MF_MT_DEFAULT_STRIDE` is set **positive** (`w*4`) to
  declare top-down data — verified by decoding frame 0 and confirming it isn't vertically
  flipped versus a live grab (the advisor flagged this as "verify, don't trust").
- **Finalize-on-drop.** Without `IMFSinkWriter::Finalize` there is no `moov` box and the file is
  unplayable, so `RecorderHandle::drop` (and `stop`) always finalize.
- **Encoder thread apartment is MTA** (`CoInitializeEx(MULTITHREADED)`), satisfying both Media
  Foundation and the WinRT WGC projections; the UI thread stays STA.
- **Verification via `IMFSourceReader`** (`dev probe-mp4`): it is the same decode stack Windows
  Media Player uses, so a successful decode + frame-0 PNG is the headless proxy for "plays in a
  real player" (actual VLC/WMP playback stays a manual-QA item).
- **Toolbar/countdown reuse the flyout window pattern** (small topmost Win32 tool window,
  `SetTimer` ticks, buttons post `Record*`/`StopRecording` actions). `AppState` gains
  `recording: Option<Recording>` and `pending_record`.

## M4 — Audio

- **Raw WASAPI for both mic and system, not cpal.** cpal 0.18 can't do loopback, so one raw
  WASAPI code path covers both: system = `AUDCLNT_STREAMFLAGS_LOOPBACK` on the default **render**
  endpoint; mic = default **capture** endpoint. No cpal dependency.
- **Silence keep-alive render client.** WASAPI loopback delivers nothing (or `SILENT` buffers)
  when no app is playing, which would starve/short the audio track. A render client on the same
  endpoint is opened alongside and fed continuously — silence in normal use, and (via the
  `RUSTSHOT_TEST_TONE_HZ` env var) a sine tone in the functional test so loopback has a known
  signal to prove the path.
- **Canonical internal format 48 kHz stereo f32**, linear-resampled per source (mic may be
  44.1 kHz), mixed by summing with a soft clamp, then converted to **PCM16** for the sink — MF's
  AAC encoder rejects float input on some systems.
- **Sample-count audio clock, held to the video clock.** Audio sample time is
  `frames_written * 10^7 / 48000`, never wall time, so packets can't jitter the timeline. But
  the video is paced by the monotonic clock and the audio by the device's own crystal, and a
  stalled or unplugged device simply stops delivering, so the two *can* drift apart. The encoder
  loop therefore compares the samples written with the samples the active (unpaused) time calls
  for: more than 200 ms behind → silence is written to close the gap (leaving 50 ms so audio
  still in flight lands on time, not early); more than 200 ms ahead (a fast device clock) → the
  excess is trimmed from the next chunk. 200 ms sits above capture latency and scheduling jitter
  and below an obvious lip-sync slip; corrections are logged. Pre-roll captured during video
  startup is discarded at the loop start so both timelines begin at t = 0 (measured skew fell
  from ~0.43 s to ~0.04 s). Pause drops audio packets, matching the paused video.
- **Per-source alignment in the mixer.** When one source starves past the wait limit, the mixer
  emits with silence in its place and records the span as *owed*; that source's late audio for
  the span is dropped when it arrives, so a microphone hiccup doesn't leave the mic permanently
  behind the system audio. A source more than 500 ms ahead of the mixer has its oldest audio
  dropped. A source whose device errors (unplugged) becomes silence for the rest of the take.
- **Any shared-mode format.** The mix format is decoded from `nBlockAlign` and the sample
  container: PCM 8/16/24/32-bit (24-in-32 is MSB-aligned, so it reads as 32-bit) and float
  32/64; anything else is refused with a note instead of silently recording zeros. More than two
  channels are folded to stereo by `dwChannelMask` (centre and surrounds at -3 dB, LFE dropped;
  dropping the centre would lose the dialogue of a 5.1 mix). The resampler interpolates across
  buffer boundaries. `GetMixFormat` memory is freed (`CoTaskMemFree` guard).
- **Microphone choice.** `microphone_device` is matched against the active capture endpoints:
  the endpoint id, else the friendly name (case-insensitive), else the first name containing it.
  No match → the default microphone, with a note. `dev list-audio-inputs` prints the names; there
  is no picker in Settings yet.
- **Graceful degradation, reported.** Each source is opened independently: a missing or busy
  microphone no longer discards a working system-audio capture. Device setup overlaps the wait
  for the first video frame and may take up to 10 s (a Bluetooth headset switching profile);
  past that the audio thread is detached and the recording is video-only. A recording that
  stops before setup reported in (the video failed to start) waits 2 s, then detaches too, so
  stopping can't hang on a device stuck in activation. Every such case leaves a note on the
  recording, and the app shows them in one dialog after the file is saved ("…saved, but its
  sound was not recorded as set up"), not mid-recording, where a dialog would land in the video.
- **AAC muxed via the same `IMFSinkWriter`** (`AddStream` before `BeginWriting`), 48 kHz stereo,
  192 kbps (`AVG_BYTES_PER_SECOND = 24000`; the encoder only accepts 12k/16k/20k/24k).
- **Verification by signal, not ears** (`dev record-audio-test` + extended `probe-mp4`): decode
  the audio stream and assert RMS > 0.1 (present, not silence) and a zero-crossing dominant
  frequency near the rendered tone. Lip-sync by ear and live mic input remain manual QA.

## M5 — GIF

- **Reuse the WGC `CaptureStream`, encode with the `image` crate.** A separate `gif::GifSession`
  (mirroring the video `Session` API) captures frames, downscales each with
  `imaging::thumbnail` (longer edge → `gif_max_width`, never upscaling), and encodes a looping
  GIF via `image`'s `GifEncoder`. The `image` dependency gains the `gif` feature.
- **Bounded in memory.** Capture stops at `gif_fps × gif_max_seconds` frames, so the in-RAM
  frame buffer is small (e.g. 15 fps × 30 s = 450 downscaled frames) — no streaming GIF encoder
  needed.
- **One handle, two backends.** `RecorderHandle` wraps a `Backend::{Video, Gif}` enum so GIF and
  MP4 share pause/stop and the same `record_flow` UX; the tray offers both.
- **Verified by re-decoding** (`imaging::probe_gif` with an independent `GifDecoder`): a live
  capture produced a 640×360, 14-frame animated GIF from a 1080p monitor.

## M6 — Polish

- **GUI subsystem + lazy console attach.** `#![windows_subsystem = "windows"]` removes the
  console for the tray app. `attach_parent_console()` only redirects stdout/stderr to the parent
  console when stdout is *not* already an inherited handle, so the `dev` CLI and `--help` print
  from a terminal without clobbering a pipe (which is how our own functional tests capture it).
- **Run-at-startup re-writes the path.** Enabling always writes `current_exe()` to the HKCU
  `Run` key (repairs a stale entry if the portable exe moved); disabling deletes it. Applied at
  startup and on settings OK.
- **Manifest + icon via `embed-resource`/rc.exe**, embedding `app.manifest` (Per-Monitor-V2 DPI,
  `asInvoker`, Win10/11 supportedOS) and an `.ico` generated from the programmatic tray icon.
  `SetProcessDpiAwarenessContext` is kept as well so the un-manifested dev/test binaries are
  still DPI-correct. Resource embedding is optional so a toolchain without rc.exe still builds.
- **Settings dialog from an in-memory `DLGTEMPLATE`, not egui.** eframe wants the main-thread
  event loop, which already belongs to the tray pump + nested overlay/dialog loops — three event
  loops would fight. A classic dialog template gives tab order, the default button, and
  Enter/Esc for free via `DialogBoxIndirectParamW`. The template builder (`DlgBuilder`) is pure
  and unit-tested (the testable core for M6); the dialog itself is visual/manual-QA.
- **Apply-on-OK re-registers hotkeys**, so the `Hotkeys` handle moved from `start()`'s stack into
  `AppState`. Re-registration drops the old manager *first* (unregistering its keys) before
  creating the new one, avoiding a self-conflict on unchanged bindings.
- **Folder picker via `rfd`** (already a workspace dep; wants STA, which the UI thread is).
