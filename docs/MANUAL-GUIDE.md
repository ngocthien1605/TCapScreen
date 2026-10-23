# RustShot — Manual Guide

*Tiếng Việt: [MANUAL-GUIDE.vi.md](MANUAL-GUIDE.vi.md)*

This manual walks through RustShot task by task: installing it, taking and annotating
screenshots, recording video and GIFs, and changing settings. For a compact reference (every
`config.toml` key, the full list of shortcut key names), see the [User Guide](USER-GUIDE.md).

Applies to **RustShot 0.1.7**.

---

## 1. What RustShot does

RustShot is a small Windows app that lives in the system tray (the icons next to the clock). It
has no main window. It does two things:

- **Screenshots**: a region you drag, one window, one monitor, or every monitor at once.
- **Screen recordings**: a region or window recorded to **MP4** video (optionally with
  microphone and computer sound), or to an animated **GIF**.

Everything starts from the tray menu or from a global keyboard shortcut.

## 2. Install

RustShot is portable: there is no installer and it does not need administrator rights.

1. Open the [Releases page](https://github.com/ngocthien1605/rustshot/releases) and download
   `RustShot-<version>-portable-x64.zip` from the newest release.
2. *(Optional)* Check the download: next to the zip is a `.sha256` file holding its SHA-256
   hash. In PowerShell, `Get-FileHash .\RustShot-0.1.7-portable-x64.zip` must print the same
   value (letter case doesn't matter).
3. Unzip it to a folder that will stay put, for example `%LOCALAPPDATA%\Programs\RustShot`.
   The zip holds `rustshot.exe`, `USER-GUIDE.md` and `LICENSE`.
4. Double-click `rustshot.exe`.

**"Windows protected your PC".** The exe is not code-signed yet, so Windows SmartScreen may
warn the first time. Choose **More info → Run anyway**.

**Requirements:** Windows 10 version 1903 or later, or Windows 11, 64-bit.

## 3. First start

- A **teal rounded-square icon** appears in the tray. If you don't see it, click the **^**
  arrow next to the clock; you can drag the icon onto the taskbar to keep it visible.
- RustShot creates its settings file, `%APPDATA%\RustShot\config.toml`, and the folder your
  captures go to, by default **`Pictures\RustShot`** (inside OneDrive if your Pictures folder is
  synced to OneDrive).
- Only one copy runs at a time. Starting `rustshot.exe` again while it is running does nothing.
- Hover over the icon to see its tooltip. **"RustShot — idle"** means every shortcut works; a
  message about unavailable hotkeys means another program already owns some of them (see
  [§10](#10-keyboard-shortcuts)).

To quit: tray icon → **Quit RustShot**.

## 4. The tray menu

Click the tray icon (left or right button):

| Menu item | What it does |
|---|---|
| **Screenshot ▸ Screenshot — Region** | Drag a rectangle, or click a window, to capture it. |
| **Screenshot ▸ Screenshot — Window** | Click a window to capture just that window. |
| **Screenshot ▸ Screenshot — Monitor** | Capture the whole monitor the mouse is on. |
| **Screenshot ▸ Screenshot — All monitors** | Capture every monitor as one image. |
| **Record ▸ Record video (region/window)** | Record a region or window to MP4. |
| **Record ▸ Record GIF (region/window)** | Record a region or window to an animated GIF. |
| **Open output folder** | Open the folder your captures are saved in. |
| **Settings…** | Open the Settings window ([§9](#9-settings)). |
| **Quit RustShot** | Exit. A recording in progress is stopped and saved first. |

## 5. Take a screenshot

### Region (default shortcut `PrintScreen`)

1. Press `PrintScreen`, or tray → **Screenshot ▸ Screenshot — Region**.
2. The screen freezes and dims. A white crosshair follows the mouse, and the window under the
   mouse is outlined in red.
3. Either **drag** a rectangle (its size, `W × H`, is shown next to the pointer) and release, or
   **click** once to take the outlined window.
4. To cancel, press `Esc` or right-click. `Enter` takes the rectangle you have drawn.

### Window (`Ctrl+PrintScreen`)

The same frozen screen appears; click the window you want. RustShot then captures the window
itself, so other windows lying on top of it are normally left out of the picture.

### Monitor (`Alt+PrintScreen`) and all monitors (`Shift+PrintScreen`)

These capture straight away, with no selection step: the monitor the mouse is on, or the whole
desktop across all monitors.

### What happens after a capture

- The image is saved in the output folder as `RustShot_YYYY-MM-DD_HH-mm-ss.png` (or `.jpg` if
  you chose JPEG in Settings). An existing file is never overwritten.
- It is copied to the clipboard, ready to paste (unless you turned that off in Settings). This
  **replaces whatever was on the clipboard**.
- A small **preview panel** opens in the bottom-right corner of the screen for about 8 seconds.
  It stays open while the mouse is over it.

| Button | Action |
|---|---|
| **Annotate** | Open the image in the editor ([§6](#6-annotate-a-screenshot)). |
| **Open** | Open it in your default image viewer. |
| **Folder** | Open Explorer with the file selected. |
| **Copy** | Copy the image to the clipboard again. |
| **Path** | Copy the file's full path as text. |
| **Delete** | Delete the file and close the panel. |

**Delayed screenshot.** To capture a menu or tooltip that closes when you press a key, set
`delay_seconds` in `config.toml` (0–60, see [§11](#11-advanced-editing-configtoml)). RustShot
then waits that many seconds before capturing. Nothing is shown while it waits.

## 6. Annotate a screenshot

1. In the preview panel, click **Annotate**. The editor opens with your screenshot.
2. Choose a tool from the toolbar. Both draw in red.
   - **Rectangle**: drag over the image.
   - **Text**: drag a box, then type. **Enter** places the text; **Shift+Enter** or
     **Alt+Enter** starts a new line; **Backspace** deletes; **Esc** throws the text away;
     clicking elsewhere also places it.
3. **Undo** (`Ctrl+Z`) removes the last mark.
4. **Save** (`Ctrl+S`) writes a copy next to the original named `…_annotated.png`; saving again
   makes `…_annotated_2.png`, and so on. **The original screenshot is never changed.**
5. **Copy** (`Ctrl+C`) puts the annotated image on the clipboard.

Closing the editor with unsaved marks asks whether to save first. Marks are drawn at the
screenshot's full resolution, however small the editor window is.

## 7. Record a video (MP4)

1. *(Optional)* Turn on sound in **Settings…** first ([§9](#9-settings)): **Record
   microphone**, **Record system audio** (what your computer is playing), or both.
2. Press `Ctrl+Shift+R`, or tray → **Record ▸ Record video (region/window)**.
3. Drag a region or click a window, exactly as for a screenshot. `Esc` cancels.
4. After a **3-second countdown** (nothing is shown on screen during it), recording starts and
   a small toolbar appears, always on top, showing the elapsed time:
   - **Pause / Resume**: paused time is left out of the video.
   - **Stop**: finishes the file.
5. Press **Stop** or `Ctrl+Shift+S`. The MP4 (H.264 video, AAC sound) is saved in the output
   folder as `RustShot_YYYY-MM-DD_HH-mm-ss.mp4`.

**Which microphone?** In Settings, **Microphone** chooses it: **System default** follows
Windows' default input device; picking a specific microphone uses it whatever Windows' default
is. The choice is made when a recording starts. See [§9](#9-settings).

**When a recording ends by itself.** If the window you are recording closes, or the graphics
driver restarts, the toolbar disappears, everything captured so far is saved as a playable
file, and a message says where it is. Resizing the recorded window is fine: the video keeps
its starting size.

**When sound is missing.** If a microphone or the system sound can't be recorded (not
connected, unplugged during the recording, a Bluetooth headset dropping out), the video is
still saved with whatever sound could be recorded, and a message afterwards lists what is
missing and why.

## 8. Record a GIF

1. Tray → **Record ▸ Record GIF (region/window)** (there is no default shortcut).
2. Drag a region or click a window. After the countdown, the toolbar appears.
3. Press **Stop**. The GIF is built and saved as `RustShot_YYYY-MM-DD_HH-mm-ss.gif`.

GIFs are kept small on purpose: by default at most **800 px** on the longer side, **15 frames
per second**, and **30 seconds** long. At the length limit the recording stops by itself and a
message tells you so. GIFs have no sound. These limits can be changed in `config.toml`
(`gif_max_width`, `gif_fps`, `gif_max_seconds`).

## 9. Settings

Tray → **Settings…**. The window has its own taskbar button. Changes apply when you press
**OK** (hotkeys are re-registered and the output folder changes immediately); **Cancel**
discards them.

| Setting | Meaning |
|---|---|
| **Include cursor in screenshots** | Draw the mouse pointer into screenshots. Recordings include the pointer by default (`recording.include_cursor`). |
| **Copy screenshots to clipboard** | Copy every screenshot to the clipboard as well as saving it. |
| **Record microphone** | Record a microphone into videos. |
| **Microphone** | Which microphone (greyed out while Record microphone is off), described below. |
| **Record system audio** | Record what the computer is playing into videos. |
| **Run RustShot at startup** | Start RustShot when you sign in to Windows. |
| **Save screenshots as JPEG (else PNG)** | JPEG files are smaller; PNG is lossless. |
| **JPEG quality (1-100)** | Used for JPEG screenshots; 90 is the default. |
| **Video frame rate (fps)** | Frames per second for MP4 recordings; 30 is the default. |
| **Output folder** | Where captures are saved. **Browse…** picks a folder. Empty means `Pictures\RustShot`. |
| **Global hotkeys** | One field per action ([§10](#10-keyboard-shortcuts)). |
| **Restore default hotkeys** | Put all six shortcuts back to the defaults. |

**The Microphone list** shows:

- **System default (name)**: whichever input Windows uses as its default, named in brackets.
  Windows sets this under *Settings → System → Sound → Input*. Note that apps such as Teams or
  Zoom often use the separate "default *communication* device", so the microphone they use may
  not be the one RustShot picks here.
- **Each connected microphone by name.** Choose one to always record from it.
- **A microphone you chose earlier that isn't connected now** stays selected, marked **(not
  connected)**. Until you plug it back in, recordings use the default microphone and say so
  afterwards.

**Run at startup** remembers where `rustshot.exe` is. If you move the folder, turn the option
off and on again.

## 10. Keyboard shortcuts

| Action | Default |
|---|---|
| Region screenshot | `PrintScreen` |
| Window screenshot | `Ctrl+PrintScreen` |
| Monitor screenshot | `Alt+PrintScreen` |
| All-monitors screenshot | `Shift+PrintScreen` |
| Start recording (video) | `Ctrl+Shift+R` |
| Stop recording | `Ctrl+Shift+S` |

**To change one:** Settings → **Global hotkeys** → click **Modify** next to the action, press
the new combination (`Backspace` clears it, `Enter` accepts, `Esc` cancels), then **OK**. You can
also type it, for example `Ctrl+Alt+F9`; leave a field empty to turn that shortcut off. Two
actions can't share a combination, and a name RustShot doesn't know is refused when you press
OK, so you can fix it before anything is saved.

**If a shortcut does nothing,** another program owns it. On Windows 11 the `PrintScreen` key
usually opens the Snipping Tool: turn that off under *Settings → Accessibility → Keyboard → Use
the Print screen key to open screen capture*, or quit the other screenshot tool, or pick
different shortcuts in RustShot. The tray tooltip shows how many shortcuts are unavailable, and
Settings tells you if a new one is taken too. The tray menu always works.

## 11. Advanced: editing config.toml

Some options exist only in `%APPDATA%\RustShot\config.toml`:

| Key | Default | Meaning |
|---|---|---|
| `screenshot.delay_seconds` | `0` | Wait before a screenshot (0–60 s). |
| `recording.countdown_seconds` | `3` | Countdown before a recording (0–30 s). |
| `recording.include_cursor` | `true` | Draw the mouse pointer into recordings. |
| `recording.bitrate` | `8000000` | Video quality in bits per second. |
| `recording.gif_max_width` | `800` | Longer side of a GIF, in pixels (16–7680). |
| `recording.gif_fps` | `15` | GIF frames per second (1–50). |
| `recording.gif_max_seconds` | `30` | GIF length limit (1–600 s). |

To edit it safely:

1. **Quit RustShot** (tray → Quit RustShot). RustShot reads the file when it starts, and saves
   it when you press OK in Settings, which would overwrite edits made in the meantime.
2. Open the file in Notepad, change the value, save.
3. Start RustShot again.

Values out of range are put back to safe defaults. If the file can no longer be read at all,
RustShot renames it to `config.toml.broken-<number>`, starts with default settings, and tells
you. Every key is listed in the [User Guide](USER-GUIDE.md#configtoml-keys).

## 12. Where RustShot keeps things

| What | Where |
|---|---|
| Screenshots and recordings | `Pictures\RustShot` (or the folder chosen in Settings) |
| Settings | `%APPDATA%\RustShot\config.toml` |
| Logs (the newest 14 days) | `%APPDATA%\RustShot\logs\rustshot.log.YYYY-MM-DD` (dates in UTC) |
| Crash details, if any | `%APPDATA%\RustShot\logs\panic.log` |
| Start-at-sign-in entry | Registry `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value `RustShot` |

To open a folder starting with `%APPDATA%`, paste the path into the Explorer address bar.

## 13. Troubleshooting

| Problem | What to do |
|---|---|
| A shortcut does nothing | Another app owns it: see [§10](#10-keyboard-shortcuts). |
| The recording used the wrong microphone | Pick it under **Settings → Microphone**, or change Windows' default input. |
| A recording has no sound | Check **Record microphone** / **Record system audio** in Settings. If a device failed, the message after the recording says which one and why. |
| The microphone sounds quiet | Usually the wrong microphone (for example the laptop's own instead of a headset's). Pick the right one in Settings. |
| A screenshot is completely black | The window shows protected (DRM) video, which Windows blanks out of captures. The file is still saved and the log notes it. |
| "The configured output folder is not usable" | The folder in Settings can't be created (missing drive, no permission). Captures go to `Pictures\RustShot` until you choose another folder. |
| "The settings file could not be read" | `config.toml` had a typo. It was moved to `config.toml.broken-<number>`; copy what you need back through Settings, then delete it. |
| Anything else failed | RustShot shows a message with the reason. The log, `%APPDATA%\RustShot\logs`, has the details; include it when reporting a problem. |

Report problems on the [GitHub issues page](https://github.com/ngocthien1605/rustshot/issues).

## 14. Update to a new version

1. Tray → **Quit RustShot**.
2. Download the new zip and replace `rustshot.exe` in your RustShot folder.
3. Start it. Your settings, shortcuts and captures are kept (they live outside the program
   folder).

## 15. Uninstall

1. If **Run RustShot at startup** is on, turn it off in Settings and press **OK**.
2. Tray → **Quit RustShot**.
3. Delete the RustShot program folder.
4. *(Optional)* Delete `%APPDATA%\RustShot` to remove settings and logs. Your screenshots and
   recordings in `Pictures\RustShot` are not touched; delete them yourself if you want.
