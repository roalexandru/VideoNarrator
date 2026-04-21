# Changelog

## v0.8.2 — Container-duration fix for audio-longer-than-video sources

### Fixed
- **Narration overflowing visual content.** `probe_video` now reads the video stream's own `duration` rather than the container's `format.duration`. For source files whose audio track outlives the picture (e.g. a previously-narrated Narrator export where the last frame was held while narration continued), the old path reported the audio length as the video length — the AI then generated narration spanning the whole inflated timeline, and Export froze the final frame for minutes while the extra audio played. Script generation is now bounded to the actual visual duration.
- **Stale cached durations on project load.** Projects saved before this fix stored the inflated duration in `video_metadata`. On load, the frontend now re-probes in the background and repairs the cache when the fresh value disagrees by > 0.5 s. When a saved script is longer than the corrected video, a toast prompts regeneration.
- **Review banner now flags past-end segments.** `predictExport` returns a new `segmentsPastEnd` count, and the Review banner shows a distinct "scheduled past the end of the video — regenerate narration" message instead of the milder "will speed up slightly" when segments start after the video's visual end.

## v0.3.0 - Settings Revamp & New Providers

### New Providers
- **Google Gemini** AI provider with Gemini 2.5 Flash and Gemini 2.5 Pro models
- **Microsoft Azure TTS** voice provider with neural narration styles and 10+ default voices across 5 languages

### Settings Redesign
- Redesigned Settings into a tabbed control center (Providers, AI, Voice) with fixed dimensions
- Compact inline provider rows with status dots replacing verbose card layout
- AI provider and model selection moved to dedicated Settings tab
- Voice/TTS configuration moved from Export step to dedicated Settings tab
- Deep-linking: [Configure] buttons in Configuration and Export steps open the correct Settings tab
- Preferences (telemetry, legal links) merged into Providers tab

### Improvements
- Configuration step shows clean summary cards for AI and Voice settings
- Export step shows read-only voice summary with quick access to Settings
- TTS provider dispatch supports both ElevenLabs and Azure in export pipeline
- SSML injection protection in Azure TTS (XML-escape all attributes)

## Unreleased

- Add CI/CD: PR quality gate and tag-triggered release pipelines
- Add 90 component and store tests with coverage reporting

## v0.1.0 - Initial Release

- 6-step wizard workflow: Project Setup, Edit Video, Configuration, Processing, Review, Export
- Video import and native screen recording (macOS screencapture, Windows ffmpeg)
- Non-linear video editor with trim, split, reorder, speed, and frame-skip controls
- Multi-provider AI narration generation (Claude and OpenAI) with vision-based frame analysis
- Context document support (Markdown, TXT, PDF) for domain-aware narration
- Six built-in narration styles: Executive, Product Demo, Technical, Teaser, Training, Critique
- Script translation to additional languages via AI
- Export to JSON, SRT, VTT, TXT, Markdown, and SSML formats
- ElevenLabs text-to-speech integration with per-segment and full-audio modes
- Audio-video merge with replace or mix modes
- Project library with persistence and version history
- Keyboard shortcuts for timeline editing
