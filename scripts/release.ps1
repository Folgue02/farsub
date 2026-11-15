# Build the Windows release archive on a Windows machine, into dist\<version>\.
#
#   powershell -ExecutionPolicy Bypass -File scripts\release.ps1 [-AllowDirty]
#
# Needs Rust (rustup, MSVC toolchain) and Visual Studio Build Tools ("Desktop
# development with C++"). Makes farsub-<version>-windows-x86_64.zip (farsub.exe and
# README.md) and SHA256SUMS.txt.
param([switch]$AllowDirty)
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.*)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value
if (-not $version) { throw "no version in Cargo.toml" }

if (-not $AllowDirty -and (git status --porcelain)) {
    throw "Uncommitted changes: commit them first (or pass -AllowDirty to test)."
}
if (-not ((git tag --points-at HEAD) -contains "v$version")) {
    Write-Warning "HEAD isn't tagged v$version (scripts/bump-version.sh makes the tag)."
}

$target = "x86_64-pc-windows-msvc"
cargo build --release --locked --target $target
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

$out = "dist\$version"
if (Test-Path $out) { Remove-Item -Recurse -Force $out }
$name = "farsub-$version-windows-x86_64"
$stage = Join-Path $out $name
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item "target\$target\release\farsub.exe", README.md $stage
$zip = Join-Path $out "$name.zip"
Compress-Archive -Path $stage -DestinationPath $zip
Remove-Item -Recurse -Force $stage

$hash = (Get-FileHash -Algorithm SHA256 $zip).Hash.ToLower()
"$hash  $name.zip" | Out-File -Encoding ascii (Join-Path $out "SHA256SUMS.txt")
Write-Host "farsub $version -> $zip"
