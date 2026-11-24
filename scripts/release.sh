#!/usr/bin/env bash
# Build FarSub release archives for GitHub Releases, into dist/<version>/.
#
#   scripts/release.sh                 # every platform this machine can build
#   scripts/release.sh macos           # only macOS
#   scripts/release.sh windows         # only Windows
#   scripts/release.sh linux           # only Linux (this machine's architecture)
#   scripts/release.sh --allow-dirty   # build even with uncommitted changes (testing)
#
# What it makes:
#   farsub-<version>-macos-<arch>.zip     FarSub.app (arm64, x86_64 or universal)
#   farsub-<version>-windows-x86_64.zip   farsub.exe
#   farsub-<version>-linux-<arch>.tar.gz  farsub
#   SHA256SUMS.txt
#
# macOS builds need a Mac. A universal (Apple Silicon + Intel) app needs rustup with
#   rustup target add aarch64-apple-darwin x86_64-apple-darwin
# otherwise only this Mac's architecture is built.
#
# Windows builds run natively on Windows (Git Bash, or scripts/release.ps1), or are
# cross-compiled from macOS/Linux with cargo-xwin:
#   rustup target add x86_64-pc-windows-msvc
#   cargo install --locked cargo-xwin   # downloads the MSVC SDK on first use
#
# Linux builds run on Linux (needs a C compiler and pkg-config).
#
# .github/workflows/release.yml runs this on GitHub for each platform when a v* tag is
# pushed and publishes the pre-release.
#
# Signing (optional): MACOS_SIGN_IDENTITY="Developer ID Application: …" signs the app
# with your certificate; otherwise it's ad-hoc signed (runs, but Gatekeeper warns on
# other Macs until it's signed and notarized).
set -euo pipefail

cd "$(dirname "$0")/.."

# Use rustup's toolchain when it's installed but not on PATH (Homebrew's keg-only rustup
# next to Homebrew's rust), plus Homebrew's llvm / lld for cross-compiling to Windows.
# Only for this script: the shell's own setup is left alone.
if command -v brew >/dev/null; then
    for f in rustup llvm lld; do
        d="$(brew --prefix "$f" 2>/dev/null)/bin"
        [ -d "$d" ] && PATH="$d:$PATH"
    done
fi
PATH="$HOME/.cargo/bin:$PATH"
if command -v rustup >/dev/null; then
    # rustup's proxies (cargo, rustc) must win over another Rust earlier on PATH.
    PATH="$(dirname "$(command -v rustup)"):$PATH"
fi
export PATH

allow_dirty=0
want=()
for a in "$@"; do
    case "$a" in
        --allow-dirty) allow_dirty=1 ;;
        macos|windows|linux) want+=("$a") ;;
        -h|--help) sed -n '2,32p' "$0"; exit 0 ;;
        *) echo "unknown argument: $a" >&2; exit 2 ;;
    esac
done

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
[ -n "$version" ] || { echo "no version in Cargo.toml" >&2; exit 1; }

if [ "$allow_dirty" = 0 ] && [ -n "$(git status --porcelain)" ]; then
    echo "Uncommitted changes: commit them first (or pass --allow-dirty to test)." >&2
    exit 1
fi
if ! git tag --points-at HEAD | grep -qx "v$version"; then
    echo "note: HEAD isn't tagged v$version (scripts/bump-version.sh makes the tag)." >&2
fi

host=$(uname -s)
case "$host" in
    Darwin) host=macos ;;
    MINGW*|MSYS*|CYGWIN*) host=windows ;;
    *) host=linux ;;
esac
if [ ${#want[@]} -eq 0 ]; then
    want=(windows)
    [ "$host" = macos ] && want=(macos windows)
    [ "$host" = linux ] && want=(linux windows)
fi

out="dist/$version"
rm -rf "$out"
mkdir -p "$out"
built=()
skipped=()

has_target() {
    # rustup knows which targets are installed; without rustup only the host's exists.
    if command -v rustup >/dev/null; then
        rustup target list --installed 2>/dev/null | grep -qx "$1"
    else
        [ "$1" = "$(rustc -vV | sed -n 's/^host: //p')" ]
    fi
}

build_macos() {
    if [ "$host" != macos ]; then
        skipped+=("macos: needs a Mac")
        return
    fi
    export MACOSX_DEPLOYMENT_TARGET=11.0
    local bins=() archs=()
    for t in aarch64-apple-darwin x86_64-apple-darwin; do
        if has_target "$t"; then
            echo "==> cargo build --release --target $t"
            cargo build --release --locked --target "$t"
            bins+=("target/$t/release/farsub")
            archs+=("${t%%-*}")
        fi
    done
    if [ ${#bins[@]} -eq 0 ]; then
        skipped+=("macos: no Apple target installed")
        return
    fi
    local arch
    if [ ${#bins[@]} -eq 2 ]; then
        arch=universal
    else
        arch=${archs[0]/aarch64/arm64}
        echo "note: building $arch only; for a universal app install rustup and" \
            "run: rustup target add aarch64-apple-darwin x86_64-apple-darwin" >&2
    fi

    local app="$out/FarSub.app"
    mkdir -p "$app/Contents/MacOS"
    if [ ${#bins[@]} -eq 2 ]; then
        lipo -create "${bins[@]}" -output "$app/Contents/MacOS/farsub"
    else
        cp "${bins[0]}" "$app/Contents/MacOS/farsub"
    fi
    # LSEnvironment: started from Finder/Dock it runs as the app itself instead of
    # handing off to a background copy (that's only for the shell command).
    cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>FarSub</string>
    <key>CFBundleDisplayName</key><string>FarSub</string>
    <key>CFBundleIdentifier</key><string>io.github.folgue02.farsub</string>
    <key>CFBundleExecutable</key><string>farsub</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
    <key>LSEnvironment</key>
    <dict><key>FARSUB_DETACHED</key><string>1</string></dict>
</dict>
</plist>
PLIST
    if [ -n "${MACOS_SIGN_IDENTITY:-}" ]; then
        codesign --force --options runtime --timestamp -s "$MACOS_SIGN_IDENTITY" "$app"
    else
        codesign --force -s - "$app"
    fi
    local zip="farsub-$version-macos-$arch.zip"
    ditto -c -k --keepParent "$app" "$out/$zip"
    rm -rf "$app"
    built+=("$zip")
}

build_windows() {
    local t=x86_64-pc-windows-msvc
    local cmd=()
    if [ "$host" = windows ]; then
        cmd=(cargo build)
    elif command -v cargo-xwin >/dev/null && has_target "$t"; then
        cmd=(cargo xwin build)
    else
        skipped+=("windows: build on Windows (scripts/release.ps1), or install rustup, then 'rustup target add $t' and 'cargo install --locked cargo-xwin'")
        return
    fi
    echo "==> ${cmd[*]} --release --target $t"
    "${cmd[@]}" --release --locked --target "$t"
    local stage="$out/farsub-$version-windows-x86_64"
    mkdir -p "$stage"
    cp "target/$t/release/farsub.exe" README.md "$stage/"
    local zip="farsub-$version-windows-x86_64.zip"
    (cd "$out" && if command -v zip >/dev/null; then
        zip -qr "$zip" "$(basename "$stage")"
    else
        powershell -NoProfile -Command "Compress-Archive -Path '$(basename "$stage")' -DestinationPath '$zip'"
    fi)
    rm -rf "$stage"
    built+=("$zip")
}

build_linux() {
    if [ "$host" != linux ]; then
        skipped+=("linux: needs Linux")
        return
    fi
    local t
    t=$(rustc -vV | sed -n 's/^host: //p')
    echo "==> cargo build --release --target $t"
    cargo build --release --locked --target "$t"
    local stage="$out/farsub-$version-linux-${t%%-*}"
    mkdir -p "$stage"
    cp "target/$t/release/farsub" README.md "$stage/"
    local tgz="farsub-$version-linux-${t%%-*}.tar.gz"
    tar -C "$out" -czf "$out/$tgz" "$(basename "$stage")"
    rm -rf "$stage"
    built+=("$tgz")
}

for p in "${want[@]}"; do
    "build_$p"
done

if [ ${#built[@]} -gt 0 ]; then
    (cd "$out" && if command -v shasum >/dev/null; then shasum -a 256 "${built[@]}"; else sha256sum "${built[@]}"; fi) \
        > "$out/SHA256SUMS.txt"
fi

echo
echo "FarSub $version → $out/"
for b in ${built[@]+"${built[@]}"}; do echo "  built    $b"; done
for s in ${skipped[@]+"${skipped[@]}"}; do echo "  skipped  $s"; done
[ ${#built[@]} -gt 0 ] || exit 1
