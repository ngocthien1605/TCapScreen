//! Developer/diagnostic CLI commands. These never start the tray; they let GPU/display paths
//! be exercised on a real desktop and let config/paths be inspected on any machine (incl. CI).

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use capture_core::{
    bounding_box, CaptureBackend, CaptureTarget, Frame, GrabOpts, MonitorInfo, Point, Rect,
    WindowInfo, WindowsBackend,
};
use clap::Subcommand;
use config::Settings;

#[derive(Debug, Subcommand)]
pub enum DevCmd {
    /// Load the config (repairing out-of-range values), print it, and show its file path.
    Config,
    /// Print all resolved RustShot paths.
    Paths,
    /// List connected monitors.
    ListMonitors,
    /// List capturable top-level windows.
    ListWindows,
    /// Grab a full monitor to a PNG file.
    GrabMonitor {
        /// Monitor index as reported by `list-monitors`.
        index: usize,
        /// Output PNG path.
        out: PathBuf,
    },
    /// Grab the first window whose title contains `query` (case-insensitive) to a file.
    GrabWindow {
        /// Case-insensitive substring of the window title.
        query: String,
        /// Output image path (extension picks PNG/JPEG).
        out: PathBuf,
    },
    /// Grab a region of the virtual desktop (physical px, virtual-desktop coords) to a file.
    GrabRegion {
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        out: PathBuf,
    },
    /// Grab the whole virtual desktop (all monitors) to a file.
    GrabAll {
        /// Output image path.
        out: PathBuf,
    },
    /// Load an image file and copy it to the clipboard (round-trip check).
    ClipboardImage {
        /// Image file to copy.
        file: PathBuf,
    },
    /// Record a monitor to an MP4 for N seconds (functional test of the recorder).
    RecordMonitor {
        /// Monitor index as reported by `list-monitors`.
        index: usize,
        /// Recording length in seconds.
        seconds: u64,
        /// Output .mp4 path.
        out: PathBuf,
    },
    /// Generate the app `.ico` from the programmatic icon (build-time resource).
    MakeIcon {
        /// Output .ico path.
        out: PathBuf,
    },
    /// Decode an MP4 back with Media Foundation and report size/sample-count (+ frame-0 PNG).
    ProbeMp4 {
        /// The .mp4 file to probe.
        file: PathBuf,
    },
    /// Record a monitor to an animated GIF for N seconds, then report its dimensions/frames.
    RecordGif {
        index: usize,
        seconds: u64,
        out: PathBuf,
    },
    /// Record a monitor with system audio while rendering a 1 kHz tone (tests the audio path).
    RecordAudioTest {
        index: usize,
        seconds: u64,
        out: PathBuf,
    },
    /// Record a monitor with a mid-recording pause (tests pause accounting).
    RecordPauseTest {
        index: usize,
        /// Total wall-clock seconds.
        seconds: u64,
        /// Pause at this second.
        pause_at: u64,
        /// Resume at this second.
        resume_at: u64,
        out: PathBuf,
    },
    /// List the active audio inputs (names usable as `microphone_device`).
    ListAudioInputs,
    /// Run audio capture in memory for N seconds and report levels and any problems. Nothing
    /// is written to disk.
    AudioProbe {
        /// Seconds to capture.
        seconds: f32,
        /// Capture system audio (loopback).
        #[arg(long)]
        system: bool,
        /// Capture the microphone.
        #[arg(long)]
        mic: bool,
        /// Microphone to use (endpoint id or name, as in `microphone_device`).
        #[arg(long)]
        mic_device: Option<String>,
    },
    /// Render the Settings dialog with defaults and auto-close it (smoke-tests the template).
    SettingsSelftest,
    /// Record the first window whose title contains `query` for up to N seconds. Reports an
    /// early end (window closed, device lost) distinctly from a normal stop.
    RecordWindow {
        /// Case-insensitive substring of the window title.
        query: String,
        /// Recording length in seconds (the recording ends sooner if the window closes).
        seconds: u64,
        /// Output .mp4 path.
        out: PathBuf,
    },
}

pub fn run(cmd: DevCmd) -> ExitCode {
    // Mirror the tray app's apartment (STA) so the dev commands exercise the same COM/WGC path
    // the real app uses. The capture backend's RoInitialize(MTA) attempt is tolerated.
    crate::logging::init_stderr();
    crate::init_com_sta();
    let result = match cmd {
        DevCmd::Config => cmd_config(),
        DevCmd::Paths => cmd_paths(),
        DevCmd::ListMonitors => cmd_list_monitors(),
        DevCmd::ListWindows => cmd_list_windows(),
        DevCmd::GrabMonitor { index, out } => cmd_grab_monitor(index, &out),
        DevCmd::GrabWindow { query, out } => cmd_grab_window(&query, &out),
        DevCmd::GrabRegion {
            x,
            y,
            width,
            height,
            out,
        } => cmd_grab_region(Rect::new(x, y, width, height), &out),
        DevCmd::GrabAll { out } => cmd_grab_all(&out),
        DevCmd::ClipboardImage { file } => cmd_clipboard_image(&file),
        DevCmd::RecordMonitor {
            index,
            seconds,
            out,
        } => cmd_record_monitor(index, seconds, out),
        DevCmd::MakeIcon { out } => cmd_make_icon(&out),
        DevCmd::RecordWindow {
            query,
            seconds,
            out,
        } => cmd_record_window(&query, seconds, out),
        DevCmd::ProbeMp4 { file } => cmd_probe_mp4(&file),
        DevCmd::RecordGif {
            index,
            seconds,
            out,
        } => cmd_record_gif(index, seconds, out),
        DevCmd::RecordAudioTest {
            index,
            seconds,
            out,
        } => cmd_record_audio_test(index, seconds, out),
        DevCmd::RecordPauseTest {
            index,
            seconds,
            pause_at,
            resume_at,
            out,
        } => cmd_record_pause_test(index, seconds, pause_at, resume_at, out),
        DevCmd::SettingsSelftest => cmd_settings_selftest(),
        DevCmd::ListAudioInputs => cmd_list_audio_inputs(),
        DevCmd::AudioProbe {
            seconds,
            system,
            mic,
            mic_device,
        } => cmd_audio_probe(seconds, system, mic, mic_device),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e:#}");
            ExitCode::FAILURE
        }
    }
}

// ---- shared steps -------------------------------------------------------------------------------

fn backend() -> Result<WindowsBackend> {
    WindowsBackend::new().context("error")
}

fn monitor_at(backend: &WindowsBackend, index: usize) -> Result<MonitorInfo> {
    let mons = backend.monitors().context("error enumerating monitors")?;
    let found = mons.len();
    mons.into_iter()
        .nth(index)
        .ok_or_else(|| anyhow!("no monitor at index {index} (found {found})"))
}

/// The first capturable window whose title contains `query` (case-insensitive).
fn window_matching(backend: &WindowsBackend, query: &str) -> Result<WindowInfo> {
    let needle = query.to_lowercase();
    let win = backend
        .windows()
        .context("error enumerating windows")?
        .into_iter()
        .find(|w| w.title.to_lowercase().contains(&needle))
        .ok_or_else(|| anyhow!("no window title contains {query:?}"))?;
    println!("matched window: {}", win.title);
    Ok(win)
}

fn grab(backend: &WindowsBackend, target: &CaptureTarget) -> Result<Frame> {
    backend
        .grab(
            target,
            GrabOpts {
                include_cursor: false,
            },
        )
        .context("capture failed")
}

/// Report a frame's geometry, then save it as an image (PNG/JPEG by extension).
fn save_frame(frame: &Frame, out: &Path) -> Result<()> {
    println!(
        "captured frame: {}x{}  stride={} (width*4={}, padding={})",
        frame.width,
        frame.height,
        frame.stride,
        frame.width as usize * 4,
        frame.stride.saturating_sub(frame.width as usize * 4),
    );
    let img = imaging::frame_to_rgba(frame, true).context("conversion failed")?;
    if imaging::is_near_black(&img, 8, 16) {
        eprintln!("warning: capture is almost entirely black (protected/DRM content?)");
    }
    imaging::save_image(&img, out, 90).context("save failed")?;
    println!(
        "saved {}x{} to {}",
        img.width(),
        img.height(),
        out.display()
    );
    Ok(())
}

/// A 30 fps, 8 Mb/s MP4 recording of `target` (no cursor) to `out`.
fn mp4(target: CaptureTarget, audio: recorder::AudioCfg, out: &Path) -> recorder::RecordCfg {
    recorder::RecordCfg {
        target,
        output: recorder::RecordOutput::Mp4 {
            fps: 30,
            bitrate: 8_000_000,
            audio,
            cursor: false,
        },
        out_path: out.to_path_buf(),
    }
}

fn start(cfg: recorder::RecordCfg) -> Result<recorder::RecorderHandle> {
    recorder::Recorder::new()
        .start(cfg)
        .context("could not start recording")
}

// ---- inspect ------------------------------------------------------------------------------------

fn cmd_config() -> Result<()> {
    let path = config::paths::config_file().context("error")?;
    let mut settings = Settings::load_from(&path).context("error loading config")?;
    let repaired = settings.sanitize();
    let toml_text = settings.to_toml().context("error serializing config")?;
    println!("# config file: {}", path.display());
    println!("# exists: {}", path.exists());
    if repaired {
        println!("# (note: some out-of-range values were repaired in memory)");
    }
    match settings.effective_output_folder() {
        Ok(folder) => println!("# effective output folder: {}", folder.display()),
        Err(e) => println!("# effective output folder: <error: {e}>"),
    }
    println!("# default hotkeys:");
    for (name, hk) in settings.hotkeys.entries() {
        println!("#   {name:<24} {hk}");
    }
    let conflicts = settings.hotkeys.conflicts();
    if conflicts.is_empty() {
        println!("# hotkey conflicts: none");
    } else {
        println!("# hotkey conflicts: {conflicts:?}");
    }
    println!("\n{toml_text}");
    Ok(())
}

fn cmd_paths() -> Result<()> {
    macro_rules! show {
        ($label:expr, $expr:expr) => {
            match $expr {
                Ok(p) => println!("{:<22} {}", $label, p.display()),
                Err(e) => println!("{:<22} <error: {e}>", $label),
            }
        };
    }
    show!("app_dir", config::paths::app_dir());
    show!("config_file", config::paths::config_file());
    show!("logs_dir", config::paths::logs_dir());
    show!("default_output", config::paths::default_output_folder());
    Ok(())
}

fn cmd_list_monitors() -> Result<()> {
    let mons = backend()?
        .monitors()
        .context("error enumerating monitors")?;
    println!("{} monitor(s):", mons.len());
    for (i, m) in mons.iter().enumerate() {
        println!(
            "  [{i}] {:<14} {}x{} @ ({},{})  scale {:.0}%{}",
            m.name,
            m.bounds.width,
            m.bounds.height,
            m.bounds.x,
            m.bounds.y,
            m.scale_factor * 100.0,
            if m.is_primary { "  (primary)" } else { "" }
        );
    }
    Ok(())
}

fn cmd_list_windows() -> Result<()> {
    let wins = backend()?.windows().context("error enumerating windows")?;
    println!("{} capturable window(s):", wins.len());
    for (i, w) in wins.iter().enumerate() {
        println!(
            "  [{i}] {}x{} @ ({},{})  {}",
            w.bounds.width, w.bounds.height, w.bounds.x, w.bounds.y, w.title
        );
    }
    Ok(())
}

fn cmd_list_audio_inputs() -> Result<()> {
    let inputs = recorder::list_inputs().context("error")?;
    for i in &inputs {
        let mark = if i.is_default { "*" } else { " " };
        println!("{mark} {}  [{}]", i.name, i.id);
    }
    println!("{} input(s); * = default", inputs.len());
    Ok(())
}

fn cmd_settings_selftest() -> Result<()> {
    let main_ret = crate::ui::settings::selftest();
    let cap_ret = crate::ui::settings::capture_selftest();
    if main_ret == -1 || cap_ret == -1 {
        bail!("dialog render FAILED (settings = {main_ret}, capture = {cap_ret})");
    }
    println!("settings dialog OK (result = {main_ret}); capture dialog OK (result = {cap_ret})");
    Ok(())
}

fn cmd_make_icon(out: &Path) -> Result<()> {
    let (rgba, w, h) = crate::icon::app_icon_rgba();
    let img = imaging::Rgba::from_raw(w, h, rgba).context("icon buffer size mismatch")?;
    if let Some(parent) = out.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    imaging::save_ico(&img, out).context("icon write failed")?;
    println!("wrote {}x{} icon to {}", w, h, out.display());
    Ok(())
}

fn cmd_clipboard_image(file: &Path) -> Result<()> {
    let img =
        imaging::load_rgba(file).with_context(|| format!("could not load {}", file.display()))?;
    crate::clipboard::copy_image(&img).context("clipboard copy failed")?;
    println!(
        "copied {}x{} image from {} to the clipboard",
        img.width(),
        img.height(),
        file.display()
    );
    Ok(())
}

// ---- stills -------------------------------------------------------------------------------------

fn cmd_grab_monitor(index: usize, out: &Path) -> Result<()> {
    let backend = backend()?;
    let mon = monitor_at(&backend, index)?;
    save_frame(&grab(&backend, &CaptureTarget::Monitor(mon.id))?, out)
}

fn cmd_grab_window(query: &str, out: &Path) -> Result<()> {
    let backend = backend()?;
    let win = window_matching(&backend, query)?;
    save_frame(&grab(&backend, &CaptureTarget::Window(win.id))?, out)
}

fn cmd_grab_region(rect: Rect, out: &Path) -> Result<()> {
    // Grab the whole virtual desktop, then crop: the same path the overlay uses.
    let backend = backend()?;
    let mons = backend.monitors().context("error enumerating monitors")?;
    let bbox = bounding_box(&mons.iter().map(|m| m.bounds).collect::<Vec<_>>());
    let frame = grab(&backend, &CaptureTarget::VirtualDesktop)?;
    save_frame(&frame.crop(rect.to_local(Point::new(bbox.x, bbox.y))), out)
}

fn cmd_grab_all(out: &Path) -> Result<()> {
    save_frame(&grab(&backend()?, &CaptureTarget::VirtualDesktop)?, out)
}

// ---- recording ----------------------------------------------------------------------------------

fn cmd_record_monitor(index: usize, seconds: u64, out: PathBuf) -> Result<()> {
    let mon = monitor_at(&backend()?, index)?;
    let handle = start(mp4(
        CaptureTarget::Monitor(mon.id),
        recorder::AudioCfg::default(),
        &out,
    ))?;
    println!(
        "recording {seconds}s of monitor {index} to {}…",
        out.display()
    );
    std::thread::sleep(Duration::from_secs(seconds));
    let path = handle.stop().context("recording failed")?;
    println!("saved recording to {}", path.display());
    Ok(())
}

fn cmd_record_window(query: &str, seconds: u64, out: PathBuf) -> Result<()> {
    let win = window_matching(&backend()?, query)?;
    let handle = start(mp4(
        CaptureTarget::Window(win.id),
        recorder::AudioCfg::default(),
        &out,
    ))?;
    println!("recording up to {seconds}s to {}…", out.display());
    let started = Instant::now();
    // Poll like the tray toolbar does, so an early end is noticed within a tick.
    while started.elapsed().as_secs() < seconds && !handle.is_finished() {
        std::thread::sleep(Duration::from_millis(250));
    }
    let ran = started.elapsed().as_secs_f32();
    match handle.stop() {
        Ok(path) => println!(
            "saved recording to {} after {ran:.1}s (stopped normally)",
            path.display()
        ),
        Err(recorder::RecorderError::EndedEarly { path, reason }) => println!(
            "ENDED EARLY after {ran:.1}s: {reason}\nsaved what was captured to {}",
            path.display()
        ),
        Err(e) => return Err(anyhow!(e).context("recording failed")),
    }
    Ok(())
}

fn cmd_record_pause_test(
    index: usize,
    seconds: u64,
    pause_at: u64,
    resume_at: u64,
    out: PathBuf,
) -> Result<()> {
    let mon = monitor_at(&backend()?, index)?;
    let handle = start(mp4(
        CaptureTarget::Monitor(mon.id),
        recorder::AudioCfg::default(),
        &out,
    ))?;
    println!(
        "recording {seconds}s, paused {pause_at}..{resume_at}s → {}",
        out.display()
    );
    std::thread::sleep(Duration::from_secs(pause_at));
    let _ = handle.pause();
    println!("  paused");
    std::thread::sleep(Duration::from_secs(resume_at - pause_at));
    let _ = handle.resume();
    println!("  resumed");
    std::thread::sleep(Duration::from_secs(seconds - resume_at));
    let p = handle.stop().context("stop failed")?;
    println!(
        "saved {} (expected active ≈ {}s)",
        p.display(),
        seconds - (resume_at - pause_at)
    );
    Ok(())
}

fn cmd_record_gif(index: usize, seconds: u64, out: PathBuf) -> Result<()> {
    let mon = monitor_at(&backend()?, index)?;
    let handle = start(recorder::RecordCfg {
        target: CaptureTarget::Monitor(mon.id),
        output: recorder::RecordOutput::Gif {
            fps: 15,
            cursor: false,
            max_width: 640,
            max_seconds: seconds.max(1) as u32,
        },
        out_path: out.clone(),
    })?;
    println!("recording {seconds}s GIF → {}", out.display());
    std::thread::sleep(Duration::from_secs(seconds));
    let p = handle.stop().context("stop failed")?;
    let info = imaging::probe_gif(&p).context("gif probe failed")?;
    println!(
        "saved {} — {}x{}, {} frames, plays {:.2}s",
        p.display(),
        info.width,
        info.height,
        info.frames,
        info.duration.as_secs_f64()
    );
    Ok(())
}

fn cmd_record_audio_test(index: usize, seconds: u64, out: PathBuf) -> Result<()> {
    let mon = monitor_at(&backend()?, index)?;
    // Inject a 1 kHz tone into the render keep-alive so loopback captures a known signal.
    std::env::set_var("RUSTSHOT_TEST_TONE_HZ", "1000");
    let handle = start(mp4(
        CaptureTarget::Monitor(mon.id),
        recorder::AudioCfg {
            microphone: false,
            system_audio: true,
            microphone_device: None,
        },
        &out,
    ))?;
    println!(
        "recording {seconds}s with a 1 kHz system tone → {}",
        out.display()
    );
    std::thread::sleep(Duration::from_secs(seconds));
    let (result, notes) = handle.finish();
    for n in &notes {
        println!("note: {n}");
    }
    let p = result.context("stop failed")?;
    println!("saved {}", p.display());
    Ok(())
}

fn cmd_probe_mp4(file: &Path) -> Result<()> {
    let png = file.with_extension("frame0.png");
    let path = file.to_string_lossy().to_string();
    let info = recorder::probe::probe(&path, Some(&png)).context("probe failed")?;
    println!(
        "video OK: {}x{}  samples={}  last_ts={}ms  frame0={}",
        info.width,
        info.height,
        info.sample_count,
        info.last_timestamp_100ns / 10_000,
        png.display()
    );
    match recorder::probe::probe_audio(&path) {
        Some(a) => println!(
            "audio: {}Hz {}ch  frames={}  rms={:.3}  dominant≈{:.0}Hz",
            a.sample_rate, a.channels, a.frames, a.rms, a.dominant_hz
        ),
        None => println!("audio: none"),
    }
    Ok(())
}

fn cmd_audio_probe(
    seconds: f32,
    system: bool,
    mic: bool,
    mic_device: Option<String>,
) -> Result<()> {
    let report = recorder::probe_capture(
        recorder::AudioRequest {
            system,
            mic,
            mic_device,
            tone_hz: None,
        },
        seconds,
    );
    println!(
        "capturing={}  frames={}  ({:.2}s at 48 kHz)  rms={:.4}  peak={:.4}",
        report.capturing,
        report.frames,
        report.frames as f64 / 48_000.0,
        report.rms,
        report.peak
    );
    for n in &report.notes {
        println!("note: {n}");
    }
    if !report.capturing {
        bail!("no audio source could be captured");
    }
    Ok(())
}
