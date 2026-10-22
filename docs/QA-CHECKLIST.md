# QA-CHECKLIST — RustShot (manual verification for GPU/display/UI paths)

These steps cannot run on a headless CI runner. Run them on a real Windows 11 desktop,
ideally a multi-monitor, mixed-DPI rig. Record pass/fail + notes in PROGRESS.md.

## M0 — Scaffold
- [ ] `rustshot.exe` launches to a tray icon with a context menu; no console window.
- [ ] Tray menu → Quit exits cleanly (process gone, mutex released).
- [ ] Launching a second instance does not create a second tray icon (single-instance).
- [ ] `rustshot dev config` prints the loaded config and the config file path.
- [ ] First run creates `%APPDATA%\RustShot\config.toml` and the output folder.
- [ ] Logs appear under `%APPDATA%\RustShot\logs`.

## M1 — Capture engine (stills)
- [ ] `rustshot dev list-monitors` lists every display with correct bounds + DPI/scale.
- [ ] `rustshot dev grab-monitor 0 out.png` saves a pixel-correct full-monitor PNG.
- [ ] Image is correct on a 150%/200%-scaled monitor (no stretching, correct dimensions).
- [ ] Wide image where stride ≠ width×4 is not sheared (stride handled).

## M2 — Screenshots
> Already verified by automation (no manual step needed): monitor / window / region /
> all-monitors grab, auto-naming, PNG/JPEG save, and clipboard image copy — via
> `rustshot dev {list-windows,grab-all,grab-region,grab-window,clipboard-image}` plus a
> scripted System.Drawing check. The items below are the interactive/visual ones.
- [ ] Region hotkey dims all monitors; drag shows live W×H; Enter/release captures; Esc cancels.
- [x] Selection overlay look (Region + Window screenshots, region video, region GIF): full-screen white crosshair with a dark edge follows the cursor; hovered window / dragged selection has a red border and stays bright; size label stays on top. *(Verified by screenshotting the running overlay, idle and mid-drag.)* Window screenshots use the same look (verified). Right-click cancels on release, so no context menu pops up underneath *(verified).*
- [ ] Zero-area region cancels cleanly.
- [ ] Window pick highlights the window under the cursor; click captures its bounds.
- [ ] All-monitors captures the full virtual desktop across mixed-DPI displays.
- [ ] Timer delay fires after N seconds (screenshot delay; no visible countdown by design).
- [ ] Cursor include/exclude honoured.
- [ ] Output PNG auto-named + saved to the output folder AND copied to the clipboard.
- [ ] Flyout shows a thumbnail + Open / Open folder / Copy / Copy path / Delete, all working.
- [ ] Protected/DRM window → near-black detected and a warning surfaced (not a silent black PNG).

## M3 — Recording (MP4, video-only)
- [ ] Countdown → compact always-on-top toolbar (elapsed + Pause/Stop) → Stop finalises.
- [ ] 60 s 1080p30 monitor recording plays correctly in VLC and Windows Media Player.
- [ ] Pause/resume produces correct timing (no frozen/!sped-up segments).
- [ ] Dropped-frame count logged; memory stays bounded (ring buffer) under load.
- [ ] Target window closes mid-record → recording stops and finalises a playable file.

## M4 — Audio
- [ ] Mic-only, system-only, and mic+system each mux correctly.
- [ ] A/V stays in sync (±1 frame drift over 3 min).
- [ ] "No audio device" handled cleanly (records video-only, surfaces a note).

## M5 — GIF
- [ ] Short region recording exports an optimised GIF that opens in a browser.
- [ ] Dimension/length caps enforced with a clear warning.

## M6/M7
- [ ] Settings UI edits persist and take effect.
- [x] Hotkeys section: editing a shortcut field re-registers that hotkey on OK; a blank field
      unbinds it; an invalid entry or a duplicate combination is reported and the dialog stays
      open; "Restore default hotkeys" resets all six fields. *(Driven end-to-end against the
      release build with UI automation: rebinding Monitor screenshot to Ctrl+Alt+F8 in-session
      made the new key capture a screenshot and the old key stop; invalid/duplicate entries named
      the offending row and kept the dialog open; Restore defaults + OK reset the config.)*
- [x] Hotkeys "Modify" button: capture dialog records the pressed combination (incl.
      `PrintScreen`), Alt-combos; Esc leaves the field unchanged, Backspace clears it, tapping
      Alt alone does nothing. *(Verified with real key events, including Shift+PrintScreen with
      Screenpresso running.)* Hotkeys are suspended while Settings is open and restored after
      Cancel. Still worth a human check: the Win-key combos, and PrintScreen with Windows'
      "Use the Print screen key to open screen capture" setting turned on.
- [x] Settings + capture dialog templates render without error — auto-verified by
      `rustshot dev settings-selftest` (both must report OK, not a `-1` result).
- [ ] Run-at-startup toggle adds/removes the `Run` key (opt-in).
- [ ] Installer produces a working Start-menu app on a clean Windows 11 VM; portable zip runs.
