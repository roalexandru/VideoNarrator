#!/usr/bin/env bash
# Verify the narrator skill's runtime dependencies are in place.
# Exits 0 if everything is ready, 1 otherwise. Prints remediation steps
# for anything missing.
#
# Honours NO_COLOR (https://no-color.org) and suppresses ANSI escapes when
# stdout is not a TTY (pipes, CI logs). Use `set -u` rather than `set -e`
# because each check is explicit — a failing `command -v` is expected and
# we want to keep going to report the full picture rather than bail early.

set -u

# Colour helpers: emit plain text when stdout isn't a terminal or when the
# user has opted out via NO_COLOR.
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
  red()   { printf "\033[31m%s\033[0m\n" "$*"; }
  green() { printf "\033[32m%s\033[0m\n" "$*"; }
  yellow(){ printf "\033[33m%s\033[0m\n" "$*"; }
else
  red()   { printf "%s\n" "$*"; }
  green() { printf "%s\n" "$*"; }
  yellow(){ printf "%s\n" "$*"; }
fi

status=0

echo "Checking narrator-cli..."
if command -v narrator-cli >/dev/null 2>&1; then
  # `narrator-cli --version` should work since clap's `version` attribute
  # is set, but guard against old builds that might not implement it.
  version=$(narrator-cli --version 2>/dev/null | head -n 1)
  if [ -z "$version" ]; then
    version="(version unknown — consider upgrading)"
  fi
  green "  ok: $(command -v narrator-cli) ($version)"
else
  red   "  missing: narrator-cli not on PATH"
  yellow "  → see references/install.md for three install options"
  status=1
fi

echo "Checking ffmpeg..."
if command -v ffmpeg >/dev/null 2>&1; then
  green "  ok: $(command -v ffmpeg)"
else
  red   "  missing: ffmpeg not on PATH"
  yellow "  → brew install ffmpeg  (macOS) / apt install ffmpeg (Linux) / download from gyan.dev (Windows)"
  status=1
fi

echo "Checking ffprobe..."
if command -v ffprobe >/dev/null 2>&1; then
  green "  ok: $(command -v ffprobe)"
else
  red   "  missing: ffprobe not on PATH (usually shipped with ffmpeg)"
  status=1
fi

echo
if [ $status -eq 0 ]; then
  green "All checks passed. The narrator skill is ready to use."
else
  red   "Fix the issues above, then re-run this script."
fi

exit $status
