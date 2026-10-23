<#
    Build a portable RustShot zip: a release build plus the user guide and license,
    with no installer required. Output: dist/RustShot-<version>-portable-x64.zip

    Usage:  pwsh -File scripts/build-portable.ps1
#>
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

# Read the workspace version from Cargo.toml ([workspace.package] version = "x.y.z").
$version = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^\s*version\s*=\s*"([^"]+)"' |
    Select-Object -First 1).Matches.Groups[1].Value
if (-not $version) { $version = '0.0.0' }

Write-Host "Building RustShot $version (release)…"
Push-Location $root
try {
    cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

    $stage = Join-Path $root "target\portable\RustShot"
    if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
    New-Item -ItemType Directory -Force -Path $stage | Out-Null

    Copy-Item (Join-Path $root 'target\release\rustshot.exe') $stage
    Copy-Item (Join-Path $root 'docs\USER-GUIDE.md') $stage
    Copy-Item (Join-Path $root 'LICENSE') $stage

    $dist = Join-Path $root 'dist'
    New-Item -ItemType Directory -Force -Path $dist | Out-Null
    $zip = Join-Path $dist "RustShot-$version-portable-x64.zip"
    if (Test-Path $zip) { Remove-Item $zip -Force }
    Compress-Archive -Path "$stage\*" -DestinationPath $zip

    Write-Host "Portable build: $zip"
}
finally {
    Pop-Location
}
