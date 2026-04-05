# Narrator

AI-powered video narration generator. Import a video, configure your narration style, and let AI generate timed narration scripts with optional text-to-speech audio.

## Features

Narrator follows a 6-step wizard workflow:

1. **Project Setup** -- Import video files or record your screen directly
2. **Edit Video** -- Trim, split, reorder clips, adjust speed, and drop frames
3. **Configuration** -- Choose AI provider, narration style, language, and attach context documents
4. **Processing** -- Frame extraction, document processing, and AI narration generation
5. **Review** -- Preview the generated narration script with segment-level editing
6. **Export** -- Export scripts as JSON, SRT, VTT, TXT, Markdown, or SSML; generate TTS audio via ElevenLabs

## Tech Stack

- **Desktop framework:** Tauri v2 (Rust backend + webview frontend)
- **Backend:** Rust (async with Tokio)
- **Frontend:** React 19, TypeScript, Tailwind CSS, Zustand
- **Video processing:** ffmpeg / ffprobe
- **AI providers:** Claude (Anthropic) and OpenAI
- **Text-to-speech:** ElevenLabs

## Installation

Download the latest release from [GitHub Releases](https://github.com/roalexandru/VideoNarrator/releases).

### macOS

The app is not code-signed, so macOS will block it by default. After downloading the `.dmg`:

1. Open the `.dmg` and drag **Narrator.app** to `/Applications`
2. Open Terminal and run:
   ```bash
   sudo xattr -rd com.apple.quarantine /Applications/Narrator.app
   ```
3. Open Narrator from Applications

### Windows

Windows SmartScreen may show a "Windows protected your PC" warning. Click **More info** → **Run anyway** to proceed.

## Prerequisites (Development)

- [Rust](https://rustup.rs/) (stable toolchain)
- [Node.js](https://nodejs.org/) (v18+) with pnpm
- [ffmpeg](https://ffmpeg.org/) installed and available on PATH

## Getting Started

```bash
# Install frontend dependencies
pnpm install

# Run in development mode
pnpm tauri dev

# Build for production
pnpm tauri build
```

## Project Structure

```
VideoNarrator/
  src/                      # React frontend
    components/             # Reusable UI components
    features/               # Feature screens (wizard steps)
    hooks/                  # Custom React hooks
    lib/                    # Utilities and Tauri command bindings
    stores/                 # Zustand state stores
    types/                  # TypeScript type definitions
  src-tauri/                # Rust backend
    src/
      lib.rs                # Tauri app entry point
      commands.rs           # Tauri command handlers
      models.rs             # Shared data models
      ai_client.rs          # Claude and OpenAI integration
      video_engine.rs       # ffmpeg frame extraction and probing
      video_edit.rs         # Video editing operations
      doc_processor.rs      # Document context extraction
      export_engine.rs      # Script export formatters
      project_store.rs      # Project persistence
      elevenlabs_client.rs  # ElevenLabs TTS client
      screen_recorder.rs    # Native screen recording
      error.rs              # Error types
```

## License

MIT
