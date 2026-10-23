# Packaging RustShot

RustShot ships two ways: a **portable zip** (no install) and a **Windows Installer (MSI)**.
A `cargo-deny` policy gates dependency licenses/advisories.

## Portable zip

```powershell
pwsh -File scripts/build-portable.ps1
```

Produces `dist/RustShot-<version>-portable-x64.zip` containing `rustshot.exe`, `USER-GUIDE.md`,
and `LICENSE`. Unzip anywhere and run `rustshot.exe` — no installation, no admin rights. If the
user enables "Run at startup", the registry entry points at the exe's current location, so keep
the folder stable (or re-toggle the setting after moving it).

## Cutting a release

Releases are built and published by GitHub Actions (`.github/workflows/release.yml`):

1. Bump `version` in the workspace `Cargo.toml` (`[workspace.package]`), run `cargo update -w`
   so `Cargo.lock` follows, and commit.
2. Create an **annotated** tag whose message is the release notes, and push it:

   ```sh
   git tag -a v0.1.5 -m "RustShot v0.1.5 ..."
   git push origin master v0.1.5
   ```

3. The workflow re-runs the gate (fmt, clippy, tests, cargo-deny), fails unless the tag equals
   `v<workspace version>`, builds the portable zip with `scripts/build-portable.ps1`, writes
   `RustShot-<version>-portable-x64.zip.sha256` (`sha256sum -c` format), and creates the GitHub
   release with both files, titled `RustShot v<version>`. Its notes are the tag message minus
   its first line (which would repeat the title), so start the message with a one-line title.

To (re)attach the zip to a tag that already exists — e.g. one released by hand before this
workflow existed — run it from **Actions → Release → Run workflow** with that tag. If the release
already exists, its notes are kept and its assets replaced (a hand-uploaded zip is swapped for
the CI-built one, whose checksum differs). A tag older than the toolchain pin says
`channel = "stable"`; if a newer stable trips a clippy lint on that old code, enter `1.98.1` in
the run form's **toolchain** field.

CI (`ci.yml`) builds the same zip on every push to `master` and keeps it for 14 days as the
`rustshot-portable` workflow artifact, so the packaging step is tested before a tag needs it.
The MSI is not built by CI (WiX v3 is not guaranteed on the `windows-latest` image).

## MSI installer (WiX + cargo-wix)

Requires the [WiX Toolset v3](https://wixtoolset.org/) and cargo-wix:

```powershell
cargo install cargo-wix
cargo wix --nocapture
```

Produces `target/wix/RustShot-<version>-x86_64.msi`. The source is `wix/main.wxs`: a **per-user**
install (no elevation) to `%LOCALAPPDATA%\Programs\RustShot`, a Start-menu shortcut, an
Add/Remove-Programs entry with the app icon and a link to the GitHub repository, and in-place
major upgrades. Per-user matches the app: its config, logs and the optional Run-at-startup entry
are all per-user (HKCU / `%APPDATA%`), and an all-users install would have put per-user registry
data in per-machine components.

> **Not built or tested.** WiX isn't installed here and CI does not build the MSI, so this file
> has never been through `candle`/`light` or ICE validation. Before shipping one: build it, run
> the ICE checks, and on a clean Windows 11 VM verify install → Start-menu launch → upgrade over
> a running copy → uninstall (QA-CHECKLIST M7).
>
> Known gaps: uninstall does not remove the `HKCU\…\Run\RustShot` value if the user enabled
> Run-at-startup (Windows Installer has no "remove this one value on uninstall" without a
> custom action; turn the setting off before uninstalling, or the stale entry fails silently at
> logon), and an upgrade while RustShot is running asks the user to close it (no
> `util:CloseApplication`, which would need the WiX Util extension).

## Dependency policy (cargo-deny)

```powershell
cargo install cargo-deny
cargo deny check
```

`deny.toml` denies yanked crates and non-crates.io sources, restricts licenses to a permissive
allow-list, and warns on duplicate crate versions.

## Release profile

`[profile.release]` in the workspace `Cargo.toml` enables `lto = "thin"` and `strip = true` for a
small, fast binary; `debug = "line-tables-only"` keeps a `.pdb` next to the exe in
`target/release` (not shipped in the zip) so panic locations are symbolizable. Panics still
unwind (no `panic = "abort"`) and are contained at every Win32 callback boundary — see
DECISIONS.md — and recorded synchronously in `%APPDATA%\RustShot\logs\panic.log`.
