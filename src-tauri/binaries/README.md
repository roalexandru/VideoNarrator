# Tauri sidecars

This directory is populated at build time with self-contained `ffmpeg` and
`ffprobe` binaries (one pair per target triple).

## Why not committed

These binaries are 50–80 MB each and are only needed at build/run time —
checking them into git bloats clones. They're fetched by the script below
and gitignored.

## Bootstrap

```bash
# fetch everything (macOS arm64 + x86_64 and Windows x64)
./scripts/fetch-ffmpeg.sh

# or just the target you need
./scripts/fetch-ffmpeg.sh macos-arm
./scripts/fetch-ffmpeg.sh macos-intel
./scripts/fetch-ffmpeg.sh windows
```

## Why these builds

The default Homebrew `ffmpeg` formula and many distro builds drop `libass`,
which is required for the `subtitles` video filter used by burn-subtitles
export. The builds fetched here are compiled with `--enable-libass` so the
filter works out of the box:

- macOS: [OSXExperts.net](https://www.osxexperts.net/) static builds
- Windows: [BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds) GPL builds

Integrity checks (SHA-256) for macOS are pinned inside `fetch-ffmpeg.sh`.
