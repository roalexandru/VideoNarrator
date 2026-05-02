# `Narrator` — Native Desktop App Architecture Plan

A cross-platform (macOS, Windows, Linux) native desktop application built in Rust + Tauri that ingests screen recordings, context documents, and user input to produce production-ready timed narration transcripts in multiple languages and styles.

---

## 1. Technology Stack

### 1.1 Why Tauri (v2)

Tauri v2 is the Electron alternative that fits this project:
- Rust backend for video processing, file I/O, API orchestration — no Node.js runtime, no Chromium bundled
- Uses the OS-native webview (WebKit on macOS, WebView2 on Windows, WebKitGTK on Linux) — app binary is ~5-10MB vs 150MB+ Electron
- IPC bridge between Rust commands and the frontend via `#[tauri::command]` — type-safe, async
- Built-in file dialogs, drag-and-drop, system tray, auto-updater, and native menu support
- Tauri v2 adds mobile support (iOS/Android) if you ever want that path

**Frontend:** React + TypeScript + Tailwind (inside the webview). Vite as the bundler. The UI is a thin presentation layer — all heavy logic lives in Rust.

**Alternative considered:** Dioxus (pure Rust UI). More "Rust-native" but the ecosystem is immature for rich media UIs. Tauri gives you the full web component ecosystem while keeping the backend in Rust.

### 1.2 Core Dependencies (Cargo)

```toml
[dependencies]
tauri = { version = "2", features = ["dialog", "fs", "shell", "path", "drag-drop"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.12", features = ["json", "multipart", "stream"] }
base64 = "0.22"
image = "0.25"                   # frame resizing, format conversion
uuid = { version = "1", features = ["v4"] }
chrono = "0.4"
toml = "1"                       # config files
pdf-extract = "0.7"              # PDF text extraction
pulldown-cmark = "0.10"          # markdown parsing
tempfile = "3"                   # temp frame storage
blake3 = "1"                     # frame deduplication hashing
rayon = "1.10"                   # parallel frame processing
directories = "5"                # OS-appropriate config/data dirs
thiserror = "1"                  # error handling
tracing = "0.1"                  # structured logging
tracing-subscriber = "0.3"
```

**External runtime dependency:** `ffmpeg` — called as subprocess. Detected at startup, bundled optionally via Tauri's sidecar mechanism (`tauri.conf.json > bundle > externalBin`). Bundling ffmpeg as a sidecar means the user doesn't need to install it separately.

---

## 2. Application Architecture

### 2.1 High-Level Data Flow

```
┌──────────────────────────────────────────────────────────┐
│                    TAURI FRONTEND (React)                 │
│                                                          │
│  ┌─────────┐  ┌──────────┐  ┌────────┐  ┌───────────┐  │
│  │ Upload  │→ │ Settings │→ │Generate│→ │  Review   │  │
│  │ & Input │  │ & Style  │  │  (AI)  │  │  & Export │  │
│  └─────────┘  └──────────┘  └────────┘  └───────────┘  │
└────────────────────────┬─────────────────────────────────┘
                         │ IPC (invoke)
┌────────────────────────▼─────────────────────────────────┐
│                    TAURI BACKEND (Rust)                   │
│                                                          │
│  ┌──────────────┐  ┌──────────────┐  ┌───────────────┐  │
│  │ VideoEngine  │  │ DocProcessor │  │ ClaudeClient  │  │
│  │ (ffmpeg sub) │  │ (pdf, md)    │  │ (API calls)   │  │
│  └──────────────┘  └──────────────┘  └───────────────┘  │
│                                                          │
│  ┌──────────────┐  ┌──────────────┐  ┌───────────────┐  │
│  │ FrameStore   │  │ ProjectStore │  │ ExportEngine  │  │
│  │ (temp files) │  │ (state/cfg)  │  │ (json/srt)    │  │
│  └──────────────┘  └──────────────┘  └───────────────┘  │
└──────────────────────────────────────────────────────────┘
```

### 2.2 Module Breakdown

**`video_engine.rs`** — Frame extraction orchestrator
- Detects ffmpeg binary (sidecar or system PATH)
- Probes video metadata: duration, resolution, codec, fps (`ffprobe -print_format json`)
- Runs two-pass extraction: scene-change detection + fixed-interval sampling
- Resizes frames via the `image` crate (not ffmpeg — gives finer control)
- Deduplicates frames using perceptual similarity (blake3 hash on downscaled thumbnails)
- Returns `Vec<Frame>` with timestamp, path, base64, dimensions

**`doc_processor.rs`** — Context document ingestion
- Accepts: `.md`, `.txt`, `.pdf`, `.docx` (docx via basic XML extraction or `docx-rs` crate)
- Extracts plain text, preserves structure (headers, sections)
- Truncates intelligently if total context exceeds token budget (see §4.3)
- Labels each document chunk with source filename for traceability

**`claude_client.rs`** — Anthropic API orchestration
- Builds the multi-modal message payload (frames + text context)
- Manages chunked generation for long videos (see §4.4)
- Handles retries, rate limits, streaming (SSE for progress feedback to UI)
- API key stored in OS keychain via Tauri's secure storage or `keyring` crate

**`project_store.rs`** — Project persistence
- Each narration session is a "project" saved to `~/.narrator/projects/<uuid>/`
- Stores: video path reference, extracted frames, context docs, settings, generated scripts
- Enables re-generation, editing, and versioning without re-processing video

**`export_engine.rs`** — Output formatting
- Generates multiple output formats from the same internal transcript model
- JSON (structured, for programmatic consumption / TTS APIs)
- SRT/VTT (subtitle format — useful for overlay or TTS alignment)
- Plain text with timestamps (human-readable script)
- Markdown (for docs/wikis)
- SSML (Speech Synthesis Markup Language — for Azure/Google TTS with pause/emphasis control)

---

## 3. Frontend UI (React + Tailwind)

### 3.1 Screen Flow

The app follows a **wizard-style flow** with a persistent sidebar showing project state. Each step is completable independently and revisitable.

#### Screen 1: Project Setup
- Drag-and-drop (or file picker) for video file
- Video preview player with duration, resolution info
- Drag-and-drop zone for context documents (multi-file)
- Document list with remove/reorder capability
- Title input field
- Description textarea (what is this video about, what should the viewer learn)

#### Screen 2: Configuration
- **Style selector** — visual cards with description (see §3.2)
- **Language selector** — checkbox grid, primary + additional languages (see §3.3)
- **Frame extraction settings** — slider for density (light/medium/heavy), advanced toggle for scene threshold and max frames
- **Advanced panel** (collapsible):
  - Custom system prompt additions
  - Token budget allocation (context vs. output)
  - Model selection (Sonnet for speed, Opus for quality)
  - Temperature slider

#### Screen 3: Processing
- Animated progress view showing:
  1. "Extracting frames..." with thumbnail strip building in real-time
  2. "Processing documents..." with document names ticking off
  3. "Generating narration..." with streaming text appearing segment by segment
- Frame filmstrip at top so user sees what Claude sees
- Cancel button
- Estimated time remaining (based on frame count and model choice)

#### Screen 4: Review & Edit
- Two-panel layout:
  - Left: video player synced to script segments (click a segment → video seeks to that timestamp)
  - Right: editable script with timestamp markers
- Per-segment controls: edit text, adjust start/end time, delete, split, merge
- Regenerate single segment (re-sends just that frame range to Claude)
- Language tabs (if multi-language, switch between generated translations)
- Tone/style indicator per segment

#### Screen 5: Export
- Format selection checkboxes (JSON, SRT, VTT, TXT, MD, SSML)
- Per-language export toggle
- Output directory picker
- "Copy to clipboard" for quick paste into TTS tools
- Direct integration buttons (future): "Open in ElevenLabs", "Open in Descript"

### 3.2 Narration Styles (5)

Given the target context (enterprise software, developer tooling, customer demos):

| Style | Label | System Prompt Personality | Use Case |
|-------|-------|--------------------------|----------|
| `executive` | **Executive Overview** | Confident, outcome-focused, minimal jargon. Emphasizes business value, ROI, and strategic impact. Short sentences, decisive tone. | C-suite demos, earnings material, investor pitches |
| `product_demo` | **Product Demo** | Polished and enthusiastic but not salesy. Walks through features step-by-step, highlights capabilities, uses "you can" framing. Accessible to non-technical audiences. | Sales engineering demos, customer webinars, partner showcases |
| `technical` | **Technical Deep-Dive** | Precise, developer-oriented. Names UI elements, APIs, config options explicitly. Assumes the viewer has domain knowledge. Uses imperative voice ("Configure the trigger", "Set the parameter"). | Developer onboarding, internal enablement, documentation |
| `teaser` | **Teaser / Trailer** | High-energy, short punchy sentences. Focus on the "wow" moments. Creates anticipation. Uses dramatic pauses (noted as `[pause]` markers in transcript for TTS). | Social media clips, event teasers, launch announcements |
| `training` | **Training Walkthrough** | Patient, methodical, instructional. "First we'll...", "Notice how...", "This is important because...". Includes callouts for common mistakes. Assumes the viewer is following along. | Internal training, customer onboarding, certification content |

Each style maps to a distinct system prompt block (stored in `styles/` as TOML or MD files so they're editable without recompiling):

```toml
# styles/product_demo.toml
id = "product_demo"
label = "Product Demo"
description = "Polished walkthrough for customers and partners"
system_prompt = """
You are narrating a product demonstration video. Your tone is polished,
enthusiastic, and accessible. Walk the viewer through each feature
step-by-step. Use "you can" and "this lets you" framing. Highlight
capabilities without being salesy. Assume a semi-technical audience
who understands the problem domain but may not know the product.
"""
pacing = "medium"        # controls suggested words-per-segment
pause_markers = true     # insert [pause] hints for TTS
```

### 3.3 Languages (Top 5 + Extensible)

Primary set — chosen for global enterprise reach:

| Language | Code | Why |
|----------|------|-----|
| English | `en` | Default. Global business standard. |
| Japanese | `ja` | Major enterprise software market. |
| German | `de` | Strong enterprise automation market in DACH region. |
| French | `fr` | EU institutional + African francophone markets. |
| Portuguese (BR) | `pt-BR` | Large Latin American RPA market. |

**Implementation:** The primary language generates the narration first. Additional languages are produced via a second Claude call with the instruction: "Translate the following timed narration script into {language}. Preserve all timestamps, segment boundaries, and `[pause]` markers. Adapt idioms naturally — do not translate literally. Maintain the same tone and style."

**Extensible:** The language list is config-driven. Users can add any language Claude supports. The UI shows the top 5 as quick-select toggles, with a "More languages..." dropdown for the full list.

### 3.4 Additional Language Considerations
- RTL support (Arabic, Hebrew) — the export engine must handle RTL markers in SRT/VTT
- CJK line-breaking — Japanese and Chinese need different line-length heuristics for subtitle output
- SSML language tags — each language segment must declare `<lang xml:lang="ja-JP">` for correct TTS pronunciation

---

## 4. AI Pipeline (Claude Integration)

### 4.1 Prompt Architecture

The Claude API call is constructed as a layered system prompt + user message:

```
SYSTEM PROMPT:
├── Base narrator instructions (role, output format, rules)
├── Selected style block (from styles/*.toml)
├── Context documents (company messaging, product docs, glossary)
│   └── Each document wrapped in <document name="filename.md">...</document>
├── Pacing guidelines (words per segment based on style)
└── Output schema definition (strict JSON)

USER MESSAGE:
├── Video metadata (duration, resolution, frame count)
├── Title: "{user-provided title}"
├── Description: "{user-provided description}"
├── Frame images (as base64 image blocks, each labeled with timestamp)
└── "Generate the timed narration script for this video."
```

### 4.2 Output Schema

Claude is instructed to return strict JSON:

```json
{
  "title": "string",
  "total_duration_seconds": 180,
  "segments": [
    {
      "index": 0,
      "start_seconds": 0.0,
      "end_seconds": 8.5,
      "text": "Welcome to Acme Studio...",
      "visual_description": "IDE launching, splash screen visible",
      "emphasis": ["Acme Studio"],
      "pace": "slow",
      "pause_after_ms": 500,
      "frame_refs": [0, 1]
    }
  ],
  "metadata": {
    "style": "product_demo",
    "language": "en",
    "model": "claude-sonnet-4-20250514",
    "generated_at": "2026-04-03T14:00:00Z"
  }
}
```

Key fields beyond basic timestamp + text:
- `visual_description` — what's on screen (useful for accessibility, for QA, and for TTS operators to understand context)
- `emphasis` — words/phrases to stress (maps to SSML `<emphasis>` tags)
- `pace` — segment-level pacing hint (slow/medium/fast) for TTS rate adjustment
- `pause_after_ms` — silence to insert after this segment (maps to SSML `<break>`)
- `frame_refs` — which extracted frames correspond to this segment (for the review UI sync)

### 4.3 Token Budget Management

Claude's context window is finite. The app must be smart about allocation:

```
Total context budget: ~180K tokens (Sonnet) or ~200K (Opus)

Allocation:
├── System prompt + style + instructions:    ~2K tokens
├── Context documents:                       ~50K tokens (cap)
├── Frame images:                            ~80K tokens (20 frames × ~4K each)
├── User title + description:                ~500 tokens
├── Output reservation:                      ~30K tokens
└── Safety buffer:                           ~17.5K tokens
```

**Document prioritization:** If context documents exceed the 50K budget:
1. Company messaging guide gets full allocation (highest priority — defines tone)
2. Glossary gets full allocation (small, high-impact)
3. Product docs are chunked and ranked by relevance to the video title/description using a TF-IDF heuristic (computed locally in Rust, no extra API call needed)
4. Lowest-ranked chunks are dropped with a warning in the UI

**Frame budget:** If the video produces more frames than the budget allows:
1. Scene-change frames get priority (they mark transitions)
2. Fixed-interval frames fill remaining slots
3. Dropped frames are interpolated in the narration timeline (Claude is told "there is additional content between frame N and frame N+1 lasting X seconds")

### 4.4 Chunked Generation for Long Videos

Videos over ~3 minutes will likely exceed optimal frame count. Strategy:

1. **Segment the video** into chapters of 60-90 seconds each based on scene-change clusters
2. **Generate narration per chapter** with overlapping context (the last segment of chapter N is included as context for chapter N+1, ensuring smooth transitions)
3. **Stitch** the chapter outputs, resolving any timestamp overlaps
4. **Continuity pass** (optional, toggleable) — send the full stitched script (text only, no frames) back to Claude with: "Review this narration script for continuity. Fix any awkward transitions between segments. Do not change timestamps."

### 4.5 Streaming & Progress

Use Claude's SSE streaming to show generation progress in real-time:
- The Rust backend opens a streaming connection via `reqwest` + `eventsource-client`
- Partial JSON is parsed incrementally (each completed segment is pushed to the frontend via Tauri event emitter)
- The frontend renders segments as they arrive, giving immediate feedback
- If the user cancels mid-stream, partial results are preserved and usable

---

## 5. Project & State Management

### 5.1 Project Structure on Disk

```
~/.narrator/
├── config.toml                  # global settings, API key reference, defaults
├── styles/                      # editable style definitions
│   ├── executive.toml
│   ├── product_demo.toml
│   ├── technical.toml
│   ├── teaser.toml
│   └── training.toml
├── context_library/             # reusable context documents
│   ├── product_messaging_guide.md
│   └── product_glossary.md
└── projects/
    └── <uuid>/
        ├── project.json         # metadata, settings, state
        ├── frames/              # extracted JPEG frames
        │   ├── 000_00.0s.jpg
        │   ├── 001_04.2s.jpg
        │   └── ...
        ├── scripts/             # generated narration versions
        │   ├── v1_en.json
        │   ├── v1_ja.json
        │   └── v2_en.json       # after edits/regeneration
        └── exports/             # final output files
            ├── narration_en.json
            ├── narration_en.srt
            ├── narration_en.ssml
            └── narration_ja.json
```

### 5.2 Context Library

The `context_library/` folder is a persistent collection of reference documents:
- Users add documents once and reuse them across projects
- The UI shows a "Context Library" panel where docs can be toggled on/off per project
- Typical contents: brand voice guide, product documentation, competitor positioning, glossary
- Each document has metadata (last updated, token count, tags) stored in a sidecar `.meta.json`

### 5.3 API Key Management

- Stored in OS keychain via the `keyring` crate (macOS Keychain, Windows Credential Manager, Linux Secret Service)
- Never written to disk in plaintext
- The app prompts for key on first launch, validates it with a lightweight API call
- Support for multiple keys (personal vs. org) with a selector in settings

---

## 6. Export Formats

### 6.1 Structured JSON (Primary)

The canonical output. Contains all metadata, timestamps, emphasis markers, pacing hints. This is what TTS APIs consume programmatically.

### 6.2 SRT / VTT (Subtitles)

Standard subtitle formats. Useful for:
- Overlaying narration text on the video as subtitles
- Importing into video editors (Premiere, Resolve, CapCut) as text tracks
- Accessibility compliance

### 6.3 Plain Text with Timestamps

```
[0:00 - 0:08]
Welcome to Acme Studio. In this walkthrough, we'll build an
email automation from scratch in under five minutes.

[0:08 - 0:15]
We start by creating a new project. Notice how the template
selector offers pre-built starting points for common scenarios.
```

Human-readable. Paste into ElevenLabs, Descript, or hand to a voice actor.

### 6.4 SSML (Speech Synthesis Markup Language)

```xml
<speak version="1.1" xmlns="http://www.w3.org/2001/10/synthesis" xml:lang="en-US">
  <prosody rate="slow">
    Welcome to <emphasis level="moderate">Acme Studio</emphasis>.
  </prosody>
  <break time="500ms"/>
  <prosody rate="medium">
    In this walkthrough, we'll build an email automation from scratch
    in under five minutes.
  </prosody>
</speak>
```

Maps directly to Azure Cognitive Services Speech, Google Cloud TTS, and Amazon Polly. The `emphasis`, `pace`, and `pause_after_ms` fields from the transcript schema translate directly to SSML tags.

### 6.5 Markdown

For pasting into Confluence, Notion, or documentation wikis. Includes a summary header, segment table, and full script.

---

## 7. Build, Package & Distribution

### 7.1 Build Pipeline

```yaml
# GitHub Actions — triggered on tag push
matrix:
  - os: macos-latest     # → .dmg (universal binary: x86_64 + aarch64)
  - os: windows-latest   # → .msi + .exe (NSIS installer)
  - os: ubuntu-22.04     # → .deb + .AppImage

steps:
  - cargo build --release
  - pnpm install && pnpm build   # frontend
  - tauri build                  # bundles everything
```

### 7.2 ffmpeg Bundling

Tauri's sidecar mechanism bundles ffmpeg per-platform:
```json
// tauri.conf.json
{
  "bundle": {
    "externalBin": ["binaries/ffmpeg"]
  }
}
```

Pre-compiled static ffmpeg binaries (~40-80MB per platform) are committed to the repo or downloaded during CI. The app binary itself stays small (~10-15MB); ffmpeg is the bulk of the distribution.

**Alternative:** Don't bundle ffmpeg. Detect system ffmpeg at startup, show install instructions if missing. Keeps the download small but adds friction. Recommend: bundle by default, with a "use system ffmpeg" option in settings.

### 7.3 Auto-Update

Tauri v2 has built-in updater support:
- Checks a remote JSON endpoint for new versions
- Downloads differential updates
- Applies on next launch
- Can be self-hosted or use GitHub Releases as the backend

---

## 8. Additional Features Worth Building

### 8.1 Smart Chapter Detection

Before sending to Claude, analyze the frame sequence locally:
- Cluster frames by visual similarity (color histogram + edge detection)
- Detect "chapter boundaries" — moments of significant visual change
- Present these as suggested chapter markers in the UI
- Let the user adjust chapter boundaries before generation
- This improves narration quality because Claude gets coherent visual chunks

### 8.2 Frame Annotation Preview

Show extracted frames in a filmstrip with hover previews. Let the user:
- Pin/unpin specific frames (pinned frames are always sent to Claude)
- Add manual annotations ("this is the credentials dialog" → gets sent as context)
- Exclude frames (e.g., loading screens, transition animations)

### 8.3 Voice Preview (Optional TTS Integration)

For v2, add optional TTS integration directly in the app:
- ElevenLabs API: user provides API key, app generates audio per-segment
- Play audio synced with video in the review screen
- Adjusts segment timing based on actual audio duration (TTS may speak faster/slower than estimated)
- Export as WAV/MP3 per-segment or as a single merged audio track

### 8.4 Batch Processing

Queue multiple videos with the same settings:
- "Apply this style + context + language set to all 12 videos in this folder"
- Background processing with progress dashboard
- Useful for generating narration for an entire demo library or training series

### 8.5 Diff & Version Comparison

When regenerating a script:
- Show a diff view (old vs. new) per segment
- Let the user cherry-pick segments from either version
- Track version history per project

### 8.6 Template System

Save a combination of (style + context docs + language set + advanced settings) as a named template:
- "Customer Demo — EN+JA" 
- "Internal Training — Technical — All Languages"
- One-click apply to new projects

---

## 9. Development Roadmap

### Phase 1 — Core (Weeks 1-3)
- Tauri v2 project scaffold with React frontend
- Video ingestion + frame extraction (ffmpeg subprocess)
- Single-language narration generation via Claude API
- Basic review UI with editable segments
- JSON + TXT export

### Phase 2 — Polish (Weeks 4-5)
- All 5 styles with editable TOML definitions
- Context document ingestion (MD, TXT, PDF)
- Multi-language generation + language selector
- SRT/VTT/SSML export
- Project persistence + context library
- OS keychain API key storage

### Phase 3 — Advanced (Weeks 6-8)
- Chunked generation for long videos
- Streaming progress UI
- Smart chapter detection
- Frame annotation + pin/exclude
- Template system
- Batch processing

### Phase 4 — Distribution (Week 9)
- CI/CD pipeline for macOS, Windows, Linux builds
- ffmpeg sidecar bundling
- Auto-updater configuration
- Landing page + documentation

### Phase 5 — TTS Integration (Post-launch)
- ElevenLabs API integration
- In-app voice preview synced to video
- Audio export (per-segment + merged)
- Voice cloning support (use your own voice)

---

## 10. Open Questions / Decisions

| # | Question | Options | Recommendation |
|---|----------|---------|----------------|
| 1 | Bundle ffmpeg or require system install? | Bundle as sidecar vs. system dependency | Bundle — eliminates #1 support issue |
| 2 | Claude model default? | Sonnet (fast, cheap) vs. Opus (best quality) | Sonnet default, Opus toggle for final/polished scripts |
| 3 | Persist video file or just reference path? | Copy video into project dir vs. store path | Store path + hash — videos are large, don't duplicate |
| 4 | License model? | Open source vs. commercial | Open source core + optional cloud features (batch API, hosted TTS) |
| 5 | Local LLM fallback? | Support Ollama/llama.cpp for offline use | Defer to v2 — vision model quality isn't there yet for local |
| 6 | Subtitle burn-in? | Generate video with baked-in subtitles via ffmpeg | Nice-to-have for Phase 3, straightforward with ffmpeg |