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

## MSI installer (WiX + cargo-wix)

Requires the [WiX Toolset v3](https://wixtoolset.org/) and cargo-wix:

```powershell
cargo install cargo-wix
cargo wix --nocapture
```

Produces `target/wix/RustShot-<version>-x86_64.msi`. The source is `wix/main.wxs`: it installs
`rustshot.exe` to `Program Files\RustShot`, adds a Start-menu shortcut, registers an
Add/Remove-Programs entry with the app icon, and supports in-place major upgrades.

> Not built in this environment (WiX isn't installed here). The `.wxs` is authored to WiX v3
> conventions; building the MSI and verifying it on a clean Windows 11 VM (install → Start-menu
> launch → uninstall) is a release-time step (QA-CHECKLIST M7).

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
