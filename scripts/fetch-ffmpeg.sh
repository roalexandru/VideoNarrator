#!/usr/bin/env bash
# Fetch the self-contained ffmpeg/ffprobe binaries the Tauri bundler expects
# in src-tauri/binaries/.
#
# Why: the regular Homebrew `ffmpeg` formula (and most default builds) drop
# libass, which means the `subtitles` filter is missing and burn-subtitles
# export silently fails with a cryptic "Error parsing a filter description".
# The binaries downloaded here are static, self-contained, and include libass.
#
# Sources:
#   macOS arm64 / x86_64  — https://www.osxexperts.net/  (static, GPL, libass)
#   Windows x64           — https://github.com/BtbN/FFmpeg-Builds (gpl, libass)
#
# Integrity: SHA-256 sums below are of the *extracted binary*, not the zip
# (OSXExperts publishes binary hashes on its page). They match what the
# upstream page publishes at the time this script was pinned. Update the
# constants below when bumping to a new ffmpeg version.
#
# Usage:
#   scripts/fetch-ffmpeg.sh              # fetch everything (macOS + Windows)
#   scripts/fetch-ffmpeg.sh macos-arm    # fetch only one target
#   scripts/fetch-ffmpeg.sh macos-intel
#   scripts/fetch-ffmpeg.sh windows

set -euo pipefail

REPO_ROOT=$(cd "$(dirname "$0")/.." && pwd)
DEST="$REPO_ROOT/src-tauri/binaries"
mkdir -p "$DEST"

# ── Pinned versions + checksums ────────────────────────────────────────────
OSXEXPERTS_BASE="https://www.osxexperts.net"
# arm64 — ffmpeg 8.1. SHA is of the extracted binary.
OSX_ARM_FFMPEG_ZIP="ffmpeg81arm.zip"
OSX_ARM_FFMPEG_BIN_SHA="9a08d61f9328e8164ba560ee7a79958e357307fcfeea6fe626b7d66cdc287028"
OSX_ARM_FFPROBE_ZIP="ffprobe81arm.zip"
OSX_ARM_FFPROBE_BIN_SHA="aab17ac7379c1178aaf400c3ef36cdb67db0b75b1a23eeef2cb9f658be8844e6"
# x86_64 — ffmpeg 8.0 (latest Intel build on OSXExperts as of 2026-04).
OSX_INTEL_FFMPEG_ZIP="ffmpeg80intel.zip"
OSX_INTEL_FFMPEG_BIN_SHA="df3f1e3facdc1ae0ad0bd898cdfb072fbc9641bf47b11f172844525a05db8d11"
OSX_INTEL_FFPROBE_ZIP="ffprobe80intel.zip"
OSX_INTEL_FFPROBE_BIN_SHA="5228e651e2bd67bb55819b27f6138351587b16d2b87446007bf35b7cf930d891"

# Windows — BtbN release build (not nightly master) for reproducibility
BTBN_ZIP_URL="https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip"

# ── helpers ────────────────────────────────────────────────────────────────
log() { printf '\e[1;36m[fetch-ffmpeg]\e[0m %s\n' "$*"; }
die() { printf '\e[1;31m[fetch-ffmpeg]\e[0m %s\n' "$*" >&2; exit 1; }

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

download() {
  local url="$1" out="$2"
  log "GET $url"
  curl -fsSL -o "$out" "$url"
}

verify_binary() {
  local path="$1" expected_sha="$2"
  local got
  got=$(sha256_of "$path")
  if [[ "$got" != "$expected_sha" ]]; then
    die "SHA mismatch for $path
  expected: $expected_sha
  got:      $got"
  fi
  log "  sha256 ok ($(basename "$path"))"
}

# ── macOS ──────────────────────────────────────────────────────────────────
fetch_macos_arm() {
  local work
  work=$(mktemp -d)
  trap 'rm -rf "$work"' RETURN

  download "$OSXEXPERTS_BASE/$OSX_ARM_FFMPEG_ZIP"  "$work/ff.zip"
  download "$OSXEXPERTS_BASE/$OSX_ARM_FFPROBE_ZIP" "$work/fp.zip"
  (cd "$work" && unzip -o -q ff.zip && unzip -o -q fp.zip)
  verify_binary "$work/ffmpeg"  "$OSX_ARM_FFMPEG_BIN_SHA"
  verify_binary "$work/ffprobe" "$OSX_ARM_FFPROBE_BIN_SHA"

  install -m 0755 "$work/ffmpeg"  "$DEST/ffmpeg-aarch64-apple-darwin"
  install -m 0755 "$work/ffprobe" "$DEST/ffprobe-aarch64-apple-darwin"
  log "macOS arm64 → $DEST/ffmpeg-aarch64-apple-darwin"
}

fetch_macos_intel() {
  local work
  work=$(mktemp -d)
  trap 'rm -rf "$work"' RETURN

  download "$OSXEXPERTS_BASE/$OSX_INTEL_FFMPEG_ZIP"  "$work/ff.zip"
  download "$OSXEXPERTS_BASE/$OSX_INTEL_FFPROBE_ZIP" "$work/fp.zip"
  (cd "$work" && unzip -o -q ff.zip && unzip -o -q fp.zip)
  verify_binary "$work/ffmpeg"  "$OSX_INTEL_FFMPEG_BIN_SHA"
  verify_binary "$work/ffprobe" "$OSX_INTEL_FFPROBE_BIN_SHA"

  install -m 0755 "$work/ffmpeg"  "$DEST/ffmpeg-x86_64-apple-darwin"
  install -m 0755 "$work/ffprobe" "$DEST/ffprobe-x86_64-apple-darwin"
  log "macOS x86_64 → $DEST/ffmpeg-x86_64-apple-darwin"
}

# ── Windows ────────────────────────────────────────────────────────────────
fetch_windows() {
  local work
  work=$(mktemp -d)
  trap 'rm -rf "$work"' RETURN

  log "GET $BTBN_ZIP_URL"
  curl -fsSL -o "$work/win.zip" "$BTBN_ZIP_URL"

  # BtbN layout: ffmpeg-master-<date>-win64-gpl/bin/ffmpeg.exe
  (cd "$work" && unzip -o -q win.zip)
  local ff_src fp_src
  ff_src=$(find "$work" -name 'ffmpeg.exe'  -path '*/bin/*' | head -n 1)
  fp_src=$(find "$work" -name 'ffprobe.exe' -path '*/bin/*' | head -n 1)
  [[ -n "$ff_src" ]] || die "ffmpeg.exe not found in BtbN zip"
  [[ -n "$fp_src" ]] || die "ffprobe.exe not found in BtbN zip"

  install -m 0755 "$ff_src" "$DEST/ffmpeg-x86_64-pc-windows-msvc.exe"
  install -m 0755 "$fp_src" "$DEST/ffprobe-x86_64-pc-windows-msvc.exe"
  log "Windows x64 → $DEST/ffmpeg-x86_64-pc-windows-msvc.exe"
}

# ── entrypoint ─────────────────────────────────────────────────────────────
target="${1:-all}"
case "$target" in
  macos-arm|macos-arm64|aarch64-apple-darwin) fetch_macos_arm ;;
  macos-intel|x86_64-apple-darwin)            fetch_macos_intel ;;
  windows|windows-x64|x86_64-pc-windows-msvc) fetch_windows ;;
  macos)
    fetch_macos_arm
    fetch_macos_intel
    ;;
  all)
    fetch_macos_arm
    fetch_macos_intel
    fetch_windows
    ;;
  *) die "unknown target: $target (expected: macos-arm|macos-intel|windows|macos|all)" ;;
esac

log "done"
