# Subcommand reference

All commands accept the global `--progress [none|json]` flag (default `none`). See [envelope.md](envelope.md).

---

## `narrator-cli probe video --input FILE`

Reports media metadata.

**Output:**

```json
{
  "ok": true,
  "data": {
    "path": "/absolute/path/to/video.mp4",
    "duration_seconds": 123.45,
    "width": 1920,
    "height": 1080,
    "fps": 30.0,
    "codec": "h264",
    "file_size": 98765432
  }
}
```

- `file_size` is in bytes.
- `codec` is the first video stream's codec name (e.g. `h264`, `hevc`, `vp9`).
- No `has_audio` field here — this probe only reports video metadata. If you need to branch on whether the source has an audio track, call `ffprobe` directly for now (the Tauri backend has a separate `probe_has_audio_stream` helper that isn't yet exposed on the CLI).

Use this before any render to confirm the file is readable and reason about frame rates / durations.

---

## `narrator-cli render burn-subs`

Burns an SRT into a video as hard subtitles via libass.

**Args:**
- `--input VIDEO` — source MP4.
- `--srt SRT` — path to the subtitle file.
- `--output OUT` — destination MP4.
- `--style STYLE.json` *(optional)* — `SubtitleStyle` JSON. See the schema below. Pass `-` to read from stdin.

**`SubtitleStyle` schema:**

```json
{
  "font_size": 22,
  "color": "#ffffff",
  "outline_color": "#000000",
  "outline": 2,
  "position": "bottom",
  "text_transform": null,
  "max_words_per_line": null
}
```

- `color` / `outline_color` — hex RGB or RGBA. RGBA's alpha byte follows the RGB (e.g. `"#00000000"` is fully transparent black).
- `position` — `"bottom"` or `"top"`.
- `font_size` — clamped to `[8, 72]` server-side.
- `outline` — clamped to `[0, 10]`.
- `text_transform` — `"uppercase"` applies ALL CAPS. Any other value or null leaves casing alone.
- `max_words_per_line` — integer. Re-wraps each cue so no line has more than N words. Null keeps the SRT's original line breaks.

**Presets you can emit verbatim:**

*Shorts / TikTok / Reels:* `{"font_size":36,"color":"#ffffff","outline_color":"#000000","outline":4,"position":"bottom","text_transform":"uppercase","max_words_per_line":2}`

*Documentary (default):* `{"font_size":22,"color":"#ffffff","outline_color":"#000000","outline":2,"position":"bottom"}`

*Clean:* `{"font_size":18,"color":"#ffffff","outline_color":"#000000","outline":1,"position":"bottom"}`

**Output:**

```json
{"ok": true, "data": {"output_path": "final.mp4"}}
```

---

## `narrator-cli render merge-audio`

Muxes an audio track onto a video.

**Args:**
- `--video VIDEO` — source.
- `--audio AUDIO` — narration audio (MP3, WAV).
- `--output OUT` — destination.
- `--replace` *(flag, default off)* — when set, swaps the video's audio track wholesale. When off, mixes narration over the source with auto-ducking at −8 dB.

**Output:**

```json
{
  "ok": true,
  "data": {
    "output_path": "narrated.mp4",
    "fell_back_to_narration_only": false
  }
}
```

`fell_back_to_narration_only: true` means the source had no usable audio stream, so we shipped narration-only. Surface that to the user if they asked for a mix.

---

## `narrator-cli render extract-frame`

Extracts a single JPEG/PNG frame from a video at a timestamp.

**Args:**
- `--input VIDEO`
- `--at SECONDS` — float.
- `--output OUT` — `.jpg` or `.png` decides the format.

**Output:** `{"ok": true, "data": {"output_path": "frame.jpg"}}`

---

## `narrator-cli render extract-thumbnails`

Generates `count` evenly-spaced JPG thumbnails.

**Args:**
- `--input VIDEO`
- `--output-dir DIR`
- `--count N` *(default 12)*

**Output:** `{"ok": true, "data": {"paths": ["DIR/thumb_0001.jpg", ...]}}`

---

## `narrator-cli render apply-edits`

Applies a full edit plan (clips + overlay effects) in one pass.

**Args:**
- `--input VIDEO`
- `--plan PLAN.json` — a `VideoEditPlan` JSON. Pass `-` for stdin.
- `--output OUT`

**`VideoEditPlan` minimal shape:**

```json
{
  "clips": [
    {"start_seconds": 0.0, "end_seconds": 5.0, "speed": 1.0, "fps_override": null}
  ],
  "effects": []
}
```

Clip and effect structs are extensive (zoom/pan, speed, freeze-frame, spotlight, blur, text overlays). If you need the full schema, extract a plan from the desktop app's saved project JSON (`~/.narrator/projects/*/project.json`) — the shape is identical.

**Output:** `{"ok": true, "data": {"output_path": "edited.mp4"}}`

---

## `narrator-cli tts narrate`

Generates narration audio using the host OS's builtin TTS engine.

**Args:**
- `--script SCRIPT.json` — a `NarrationScript` JSON. Pass `-` for stdin.
- `--output OUT.mp3`
- `--video-duration SECONDS` *(optional)* — pads trailing silence to match. Defaults to the script's `total_duration_seconds`.
- `--voice NAME` *(default "")* — voice name. Platform-specific: macOS names (`Samantha`), Windows SAPI voices, espeak voice codes.
- `--speed FLOAT` *(default 1.0)* — playback speed multiplier.

**`NarrationScript` minimal shape:**

```json
{
  "title": "...",
  "total_duration_seconds": 60.0,
  "segments": [
    {"index": 0, "start_seconds": 0.0, "end_seconds": 3.0, "text": "..."}
  ]
}
```

**Output:**

```json
{
  "ok": true,
  "data": {
    "output_path": "narration.mp3",
    "segments_total": 10,
    "segments_compressed": 2,
    "segments_over_cap": 0
  }
}
```

- `segments_compressed` — how many segments had to be sped up via atempo to fit their scripted window.
- `segments_over_cap` — segments that needed more compression than the cap (~1.3×) allowed. These will overrun their window by the residual amount.

For richer voices (ElevenLabs, Azure), use the desktop GUI — the CLI only exposes the free builtin engine today.
