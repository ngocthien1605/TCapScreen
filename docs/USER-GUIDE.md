# RustShot — User Guide

RustShot is a lightweight Windows screenshot and screen-recording tool that lives in the system
tray. It captures regions, windows, monitors, or the whole desktop, and records video (MP4) or
animated GIFs with optional audio.

This is the compact reference. For a step-by-step walkthrough (installing, each task, updating,
uninstalling) see the Manual Guide, in [English](MANUAL-GUIDE.md) or
[Vietnamese](MANUAL-GUIDE.vi.md). This guide describes RustShot 0.1.7.

## Getting started

Run `rustshot.exe`. A tray icon (a teal rounded square) appears near the clock — RustShot has no
main window; everything is driven from the tray menu and global hotkeys. Right-click (or
left-click) the tray icon for the menu: the **Screenshot** and **Record** submenus, **Open output
folder**, **Settings…**, and **Quit RustShot** to exit (a recording in progress is stopped and
saved first).

On first run, RustShot writes its configuration to `%APPDATA%\RustShot\config.toml` and creates
the output folder (by default `Pictures\RustShot`). Logs are written to `%APPDATA%\RustShot\logs`.

Only one instance runs at a time; launching a second exits immediately.

## Taking screenshots

From the tray **Screenshot** submenu (its items read "Screenshot — Region" and so on):

| Item | What it does |
|---|---|
| **Region** | Dims the screen and draws a white crosshair (with a dark edge) across the whole screen through the cursor. The window under the cursor is outlined in red; drag a rectangle to select your own area, outlined in red. Release to capture, `Esc` to cancel, `Enter` to confirm the current rectangle. |
| **Window** | On the same overlay, click a window to capture just that window. |
| **Monitor** | Captures the monitor the mouse is on. |
| **All monitors** | Captures the entire virtual desktop across every monitor. |

In the region/window overlay, the current selection is shown undimmed with its live `W × H`
size next to the cursor. A click without dragging selects the window under the pointer.

Each screenshot is saved to the output folder with an automatic name
(`RustShot_YYYY-MM-DD_HH-mm-ss.png`), copied to the clipboard (if enabled), and shown in a small
**flyout** in the bottom-right corner with buttons:

- **Annotate** — open the screenshot in RustShot's own editor (see below); the flyout closes.
- **Open** — open the image in the default viewer.
- **Folder** — open Explorer with the file selected.
- **Copy** — copy the image to the clipboard again.
- **Path** — copy the file's full path.
- **Delete** — delete the file (and dismiss the flyout).

The flyout closes by itself about 8 seconds after it appears, but stays open while the pointer
is over it.

With `delay_seconds` set in `config.toml`, a screenshot waits that long before capturing; nothing
is shown during the wait.

### Annotating a screenshot

Click **Annotate** in the flyout to open the screenshot in RustShot's editor. It has two tools,
both in red, chosen from the toolbar:

- **Rectangle** — drag on the image to draw a red rectangle.
- **Text** — drag to draw a red box, then just type; **Enter** saves the text into the box.
  **Shift+Enter** or **Alt+Enter** starts a new line instead of saving.
  `Backspace` edits while typing, `Esc` throws the text away, and clicking elsewhere also saves it.
  The mouse cursor over the image is a red crosshair.

Other controls: **Undo** (`Ctrl+Z`) removes the last annotation (or cancels the one you're drawing);
**Save** (`Ctrl+S`) writes a copy next to the original as `…_annotated.png` — your original is
never overwritten, and saving again creates `…_annotated_2.png`; **Copy** (`Ctrl+C`) puts the
annotated image on the clipboard. Closing the editor with unsaved annotations asks whether to
save first. Annotations are drawn at the screenshot's full resolution, whatever the window size.

## Recording

From the tray **Record** submenu:

- **Record video (region/window)** — pick a region or window (the same crosshair overlay as
  Region screenshots). After the countdown (3 s by default; nothing is shown during it) recording
  starts and a compact always-on-top toolbar appears with the elapsed time, **Pause/Resume**, and
  **Stop**. Output is an MP4 (H.264 + AAC), optionally with microphone and/or system audio.
  **Stop** (or `Ctrl+Shift+S`) finalizes a playable file.
- **Record GIF (region/window)** — same selection, but produces an optimized, looping animated
  GIF (bounded by the configured max width, frame rate, and duration).

Recordings are saved to the output folder with automatic names (`.mp4` / `.gif`).

If the window you are recording is closed, or the graphics driver resets, the recording ends
on its own: the toolbar disappears, everything captured up to that point is saved as a playable
file, and a notice tells you where it is. Resizing the recorded window mid-recording is fine;
the video keeps its starting size (a larger window is cropped, a smaller one is padded).

With microphone and/or system audio turned on, each source is recorded independently: if one
cannot be used, the other still is, and a notice after the recording says what is missing.
Sound stays in step with the picture; if every audio device stalls, the gap is filled with
silence rather than letting the sound slide out of sync.

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

**Microphone** chooses which microphone "Record microphone" uses: **System default** (whatever
Windows has set as the default input, named in brackets) or any connected microphone. A
microphone you chose earlier that isn't connected right now stays selected, marked "not
connected", so it is used again when you plug it back in; until then recordings use the default
microphone and say so afterwards. The list is greyed out while "Record microphone" is off.

It also has a **Global hotkeys** section with an editable field for each of the six actions.
Either click **Modify** next to a row and press the key combination you want (including
`PrintScreen` and `Alt`/`Win` combos; Backspace clears it, Enter accepts, Esc cancels), or type
a shortcut directly as `Modifier+…+Key` (for example `Ctrl+Shift+R`, or just `PrintScreen`);
leave a field **blank** to turn that hotkey off. Supported modifiers are `Ctrl`, `Alt`, `Shift`, and
`Win`; keys include letters, digits, `F1`–`F24`, `PrintScreen`, `Space`, `Tab`, `Enter`,
`Escape`, `Backspace`, `Delete`, `Insert`, `Home`, `End`, `PageUp`, `PageDown`, the arrow keys
(`ArrowUp` or just `Up`, …), `Pause`, `ScrollLock`, `CapsLock`, `NumLock`, `ContextMenu`, the
numeric keypad (`Numpad0`–`Numpad9`, `NumpadAdd`, …) and punctuation keys by their US-layout
name (`Minus`, `Equal`, `Comma`, `Period`, `Slash`, `Semicolon`, `Quote`, `Backquote`,
`BracketLeft`, `BracketRight`, `Backslash`). A name RustShot does not know is refused on **OK**
rather than saved as a shortcut that would never fire. **Restore default hotkeys** resets all six to the
table above. On **OK**, an invalid shortcut or two actions sharing the same combination are
reported and the dialog stays open so you can fix them.

Changes are saved and take effect immediately (hotkeys are re-registered, the output folder is
updated). Everything is also editable directly in `config.toml`.

### config.toml keys

```toml
schema_version = 1           # file format version, written by RustShot; leave as is
output_folder = ""          # empty = default (Pictures\RustShot)
run_at_startup = false       # add/remove the per-user Run entry

[screenshot]
format = "png"               # "png" or "jpeg"
jpeg_quality = 90            # 1..=100
include_cursor = false
copy_to_clipboard = true
delay_seconds = 0            # wait before a screenshot (0..=60)

[recording]
fps = 30
bitrate = 8000000            # video bits per second
include_cursor = true
countdown_seconds = 3        # 0..=30
capture_microphone = false
capture_system_audio = false
microphone_device = ""       # empty = system default; else the microphone's name (or part
                             # of it); set by the Microphone list in Settings
gif_max_width = 800          # longer edge, 16..=7680; frames are scaled to fit
gif_max_seconds = 30         # 1..=600; the recording stops, with a notice, at the limit
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

- **A hotkey does nothing:** another app owns it (see the note above). The tray tooltip says
  how many hotkeys are unavailable (it updates after a change in Settings); rebind them in **Settings…** (it tells
  you if the new key is taken too) or in `config.toml`.
- **"The settings file could not be read" at startup:** `config.toml` was not valid TOML (usually
  a hand-edit typo). RustShot moved it to `config.toml.broken-<number>` next to the original,
  wrote a fresh default file, and started with default settings and hotkeys. Copy what you need
  from the `.broken` file back into Settings, then delete it. A single misspelt shortcut does
  not cause this: that shortcut simply ends up unbound.
- **"The configured output folder is not usable":** the folder in Settings could not be created
  (missing drive, no permission). Captures go to `Pictures\RustShot` until you choose another
  folder. A folder must be a full path such as `D:\Shots`; a relative name is reset to the
  default.
- **Something failed:** RustShot shows a dialog for any screenshot, recording, clipboard, delete
  or settings operation that did not complete, with the reason; the log has the details.
- **A capture is all black:** the target is DRM-protected (e.g. some video players), which
  Windows blanks out of captures. RustShot still saves what it captured and notes it in the log.
- **Sound missing from a recording:** if a microphone or system audio could not be recorded
  (no microphone connected, the one named in `microphone_device` not found, a device unplugged
  mid-way, an unsupported device format), RustShot still saves the video, with whatever sound
  it could get, and then shows a dialog listing what is missing and why. A microphone named in
  `microphone_device` that is not connected falls back to the default microphone. Otherwise,
  check the microphone and system-audio toggles in Settings.
- **Logs:** `%APPDATA%\RustShot\logs\rustshot.log.<date>` has details for any failure; the newest
  14 days are kept.
