---
name: narrator
description: Render voice-over narration onto videos, burn subtitles, probe media metadata, and run small video edits using the `narrator-cli` binary. Use whenever the user wants to narrate a video from a script, burn subtitles into an MP4, mix TTS audio with a source video, extract frames or thumbnails, or inspect video metadata outside the Narrator desktop GUI.
---

# Narrator CLI skill

Wraps `narrator-cli`, the headless interface to the same render pipeline that powers the Narrator desktop app. Every subcommand takes a single JSON envelope in and returns a single JSON envelope out, so it composes cleanly with `jq`, pipes, and shell scripting.

## When to use this skill

Use it when any of the following are true:

- The user has a video and a script and wants voice-over audio on top.
- The user wants to burn subtitles (an SRT) into a video file.
- The user wants to mix a prerecorded narration track with a source video, optionally ducking the original audio.
- The user wants metadata for a media file (fps, duration, codec, resolution).
- The user wants to extract a single frame or a thumbnail filmstrip from a video.

Do **not** reach for this skill when the user actually wants the AI-narration generation flow — that lives in the desktop GUI and is not yet exposed on the CLI.

## Prerequisites

1. `narrator-cli` must be on `PATH`. Run `scripts/doctor.sh` to verify; it prints installation instructions if the binary is missing.
2. `ffmpeg` and `ffprobe` must be on `PATH` (the CLI shells out to both).

## Envelope protocol — read first

Every invocation prints exactly one line of JSON on stdout, regardless of success or failure:

```json
{"ok": true,  "data": { ... }}
{"ok": false, "error": {"kind": "SomeError", "message": "human-readable"}}
```

Progress events are suppressed by default. Pass `--progress json` to get one NDJSON object per line on stderr while a long-running render is in flight — stdout stays reserved for the final envelope so `jq` pipelines work cleanly.

See [references/envelope.md](references/envelope.md) for the full protocol, including error shapes and how to consume NDJSON progress.

## Command reference

Full argument schemas for every subcommand are in [references/subcommands.md](references/subcommands.md). Quick index:

| Goal | Command |
|---|---|
| Inspect a video's metadata | `narrator-cli probe video --input FILE` |
| Burn an SRT into a video | `narrator-cli render burn-subs --input VIDEO --srt SRT --output OUT [--style STYLE.json]` |
| Mux/mix audio onto a video | `narrator-cli render merge-audio --video VIDEO --audio AUDIO --output OUT [--replace]` |
| Extract one frame | `narrator-cli render extract-frame --input VIDEO --at 12.5 --output frame.jpg` |
| Extract N thumbnails | `narrator-cli render extract-thumbnails --input VIDEO --output-dir DIR --count 12` |
| Apply a full edit plan | `narrator-cli render apply-edits --input VIDEO --plan PLAN.json --output OUT` |
| Generate narration audio from a script | `narrator-cli tts narrate --script SCRIPT.json --output OUT.mp3 [--voice NAME] [--speed 1.0]` |

The `tts narrate` subcommand uses the host OS's builtin TTS engine (macOS `say`, Windows PowerShell SAPI, Linux espeak) — no API keys required. The desktop app offers richer voices via ElevenLabs and Azure, which are not currently exposed on the CLI.

## Typical workflow

### Narrate a video end-to-end (script already exists)

```bash
# 1. Generate narration audio from a NarrationScript JSON
narrator-cli tts narrate --script script.json --output narration.mp3

# 2. Mix narration onto the source video (keeps original audio, ducks it under narration)
narrator-cli render merge-audio --video source.mp4 --audio narration.mp3 --output narrated.mp4

# 3. (Optional) Burn subtitles
narrator-cli render burn-subs --input narrated.mp4 --srt script.srt --output final.mp4
```

### Replace audio instead of mixing

Add `--replace` to `merge-audio` to swap the source's audio track wholesale. Use this when the source audio is unusable or when the narration is meant to stand alone.

### Error handling

Always check `.ok` on the stdout envelope. Example with `jq`:

```bash
out=$(narrator-cli probe video --input video.mp4)
if [ "$(echo "$out" | jq -r '.ok')" != "true" ]; then
  echo "probe failed: $(echo "$out" | jq -r '.error.message')" >&2
  exit 1
fi
duration=$(echo "$out" | jq -r '.data.duration_seconds')
```

### Recovery when something is misconfigured

If a `narrator-cli` invocation fails with a "not found" / "command not found" / "No such file or directory" error at the shell level (before the envelope is emitted), the binary is not installed or not on `PATH`. In that case:

1. Run `skills/narrator/scripts/doctor.sh` and surface its output to the user verbatim — it identifies exactly which of `narrator-cli`, `ffmpeg`, `ffprobe` is missing and how to install each.
2. Do not guess alternative binary names or paths; the doctor script is the source of truth.

If the envelope returns `ok: false` with a specific `error.kind`, show the `error.message` to the user as-is (the message is human-readable and doesn't need interpretation). Do not retry with different arguments unless the user asks — retry without new input is rarely the right recovery.

Treat `.data` as an open schema: parse the fields you need via `jq`/`json.loads`, don't assume the field set is frozen. Additional fields may appear in new releases; consumers that structurally match a fixed shape will break unnecessarily.

## Installation

The repo ships this skill at `skills/narrator/`. To install for Claude Code:

```bash
cd /path/to/VideoNarator            # must be the repo root
mkdir -p ~/.claude/skills
ln -s "$(pwd)/skills/narrator" ~/.claude/skills/narrator
```

Then either put `narrator-cli` on `PATH` directly or create a shim — see [references/install.md](references/install.md) for platform-specific notes.
