# RustShot — User Guide

RustShot is a lightweight Windows screenshot and screen-recording tool that lives in the system
tray. It captures regions, windows, monitors, or the whole desktop, and records video (MP4) or
animated GIFs with optional audio.

## Getting started

Run `rustshot.exe`. A tray icon (a teal rounded square) appears near the clock — RustShot has no
main window; everything is driven from the tray menu and global hotkeys. Right-click (or
left-click) the tray icon for the menu. Choose **Quit RustShot** to exit.

On first run, RustShot writes its configuration to `%APPDATA%\RustShot\config.toml` and creates
the output folder (by default `Pictures\RustShot`). Logs are written to `%APPDATA%\RustShot\logs`.

Only one instance runs at a time; launching a second exits immediately.

## Taking screenshots

From the tray **Screenshot** submenu:

| Item | What it does |
|---|---|
| **Region** | Dims the screen; drag a rectangle. Release to capture, `Esc` to cancel, `Enter` to confirm the current rectangle. |
| **Window** | On the same overlay, click a window to capture just that window. |
| **Monitor** | Captures the monitor the mouse is on. |
| **All monitors** | Captures the entire virtual desktop across every monitor. |

In the region/window overlay, the current selection is shown undimmed with its live `W × H`
size next to the cursor. A click without dragging selects the window under the pointer.

Each screenshot is saved to the output folder with an automatic name
(`RustShot_YYYY-MM-DD_HH-mm-ss.png`), copied to the clipboard (if enabled), and shown in a small
**flyout** in the bottom-right corner with buttons:

- **Open** — open the image in the default viewer.
- **Folder** — open Explorer with the file selected.
- **Copy** — copy the image to the clipboard again.
- **Path** — copy the file's full path.
- **Delete** — delete the file (and dismiss the flyout).

The flyout auto-dismisses after a few seconds.

## Recording

From the tray **Record** submenu:

- **Record video (region/window)** — pick a region or window (same overlay as screenshots),
  then recording starts (after the optional countdown) and a compact always-on-top toolbar
  appears with the elapsed time, **Pause/Resume**, and **Stop**. Output is an MP4 (H.264),
  optionally with microphone and/or system audio. **Stop** finalizes a playable file.
- **Record GIF (region/window)** — same selection, but produces an optimized, looping animated
  GIF (bounded by the configured max width, frame rate, and duration).

Recordings are saved to the output folder with automatic names (`.mp4` / `.gif`).

## Default hotkeys

| Action | Default |
|---|---|
| Region screenshot | `PrintScreen` |
| Window screenshot | `Ctrl+PrintScreen` |
| Monitor screenshot | `Alt+PrintScreen` |
| All-monitors screenshot | `Shift+PrintScreen` |
| Start region recording | `Ctrl+Shift+R` |
| Stop recording | `Ctrl+Shift+S` |

**Note:** Windows 11's "Use PrintScreen to open Snipping Tool" setting, and other screenshot
tools (Snipping Tool, ShareX, Screenpresso), may already own the `PrintScreen` combinations. If
RustShot can't register a hotkey it logs a warning and keeps running — the tray menu always
offers the same actions, and you can rebind the hotkeys in **Settings…** (or in `config.toml`).

## Settings

Tray → **Settings…** opens a dialog for the common options (cursor capture, copy-to-clipboard,
microphone/system audio, run-at-startup, JPEG quality, video FPS, and the output folder).

It also has a **Global hotkeys** section with an editable field for each of the six actions.
Either click **Modify** next to a row and press the key combination you want (including
`PrintScreen` and `Alt`/`Win` combos; Backspace clears it, Enter accepts, Esc cancels), or type
a shortcut directly as `Modifier+…+Key` (for example `Ctrl+Shift+R`, or just `PrintScreen`);
leave a field **blank** to turn that hotkey off. Supported modifiers are `Ctrl`, `Alt`, `Shift`, and
`Win`; keys include letters, digits, `F1`–`F24`, and `PrintScreen`, `Space`, `Tab`, `Delete`,
`Insert`, `Home`, `End`, `PageUp`, `PageDown`. **Restore default hotkeys** resets all six to the
table above. On **OK**, an invalid shortcut or two actions sharing the same combination are
reported and the dialog stays open so you can fix them.

Changes are saved and take effect immediately (hotkeys are re-registered, the output folder is
updated). Everything is also editable directly in `config.toml`.

### config.toml keys

```toml
output_folder = ""          # empty = default (Pictures\RustShot)
run_at_startup = false       # add/remove the per-user Run entry

[screenshot]
format = "png"               # "png" or "jpeg"
jpeg_quality = 90            # 1..=100
include_cursor = false
copy_to_clipboard = true
delay_seconds = 0            # countdown before a screenshot

[recording]
fps = 30
bitrate = 8000000            # video bits per second
include_cursor = true
countdown_seconds = 3
capture_microphone = false
capture_system_audio = false
microphone_device = ""       # empty = system default
gif_max_width = 800          # longer edge; frames are scaled to fit
gif_max_seconds = 30
gif_fps = 15

[hotkeys]
region_screenshot = "PrintScreen"
window_screenshot = "Ctrl+PrintScreen"
monitor_screenshot = "Alt+PrintScreen"
all_monitors_screenshot = "Shift+PrintScreen"
region_recording = "Ctrl+Shift+R"
stop_recording = "Ctrl+Shift+S"
```

Out-of-range values are repaired to safe defaults on load. Unknown keys are ignored, so a config
from a newer build still loads in an older one.

## Troubleshooting

- **A hotkey does nothing:** another app owns it (see the note above). Rebind it in `config.toml`.
- **A capture is all black:** the target is DRM-protected (e.g. some video players); RustShot
  warns and still saves what it captured.
- **No audio in a recording:** check the microphone/system-audio toggles; if the chosen device
  is missing, RustShot records video-only and notes it in the log.
- **Logs:** `%APPDATA%\RustShot\logs\rustshot.log.<date>` has details for any failure.
