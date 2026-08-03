# Changelog

## v0.9.5 — Current-generation models + edit → render → export hardening

### Added
- **Current-generation models across all three providers.** The picker had drifted a full generation behind (Claude Sonnet/Opus 4 — both deprecated — GPT-4o, o3, Gemini 2.5). Now offered: Claude **Sonnet 5** (new default), **Opus 5**, Haiku 4.5 and Fable 5; OpenAI **GPT-5.6 Sol / Terra / Luna**; Google **Gemini 3.6 Flash**, 3.5 Flash, 3.5 Flash-Lite and 3.1 Pro. Projects saved with older model IDs still load.
- **Reasoning depth selection** (Settings → AI). One choice — Fast / Balanced / Thorough / Maximum — mapped to whichever parameter each vendor uses (`output_config.effort`, `reasoning_effort`, `thinkingLevel`), and clamped where a provider's ladder is shorter. Deeper reasoning costs more tokens per frame analysed, so the tradeoff is stated in the UI.

### Fixed
- **Windows release builds.** The pinned ffmpeg sidecar URL 404'd because upstream deletes its dated builds after a few months, which broke every Windows release build since May. Repinned to a live build on the same 8.1 line, with docs so the next expiry is a two-minute fix.
- **Requests to current Claude models no longer fail.** Opus 4.7+ removed `temperature`/`top_p`/`top_k` (sending them is a hard 400) and thinking now shares the output-token budget with the response — the old fixed 8192 ceiling risked truncating a reply mid-JSON. Both are handled per model.

## v0.9.4 — Edit → render → export hardening

A broad correctness, safety, and lifecycle pass across the video-edit,
render, and export pipeline.

### Security
- **Path traversal hardening.** `generate_narration` and `list_project_frames` now validate the project ID as a UUID before joining it into `~/.narrator`, and `save_script` sanitizes the language before using it in a filename — a crafted `../…` value can no longer read, enumerate, or delete arbitrary directories. Also adds `.narrator` import size/entry caps, document-import preflight validation, and export basename/language sanitization.

### Fixed
- **Exports are now playable everywhere.** The compositor encoded edited video losslessly (`ultrafast -crf 0`), producing 50–150 Mbps files that QuickTime and Windows Media Foundation refuse to decode; it now uses visually-lossless `medium -crf 18`.
- **Trim-only edits are no longer dropped at export.** A split-and-delete-half edit previously exported the untrimmed original; the render decision is now a single shared predicate used by every screen.
- **Redactions reach the AI.** A blur/spotlight/text effect now forces a render before frame extraction, so the frames sent to the model contain what the user hid (previously only zoom-pan did). Deleting an effect after a render no longer exports the stale rendered file.
- **"Time-lapse" now works.** The `skipFrames` toggle was never transmitted, so a sped-up clip kept its chipmunk audio; it's now serialized and silences the clip's audio as intended.
- **Cancellation.** Edit renders, audio merges, subtitle burns, and the scene/silence detection passes are now cancellable (with Cancel buttons on the edit and export progress), instead of running to completion.
- **No more hung/leaked ffmpeg.** The compositor kills its decoder/encoder on any early exit, removes the timeline WAV on every path, and writes to a partial file that is renamed into place only on success — a crashed or cancelled encode can no longer leave a truncated video at the output path.
- **Windows recorder no longer wedges.** A failed stop always closes the overlay, resets state, keeps the recorded segments, and surfaces the error; the gdigrab process is reliably killed; and recording directories containing apostrophes work.
- **Fresh frames per generation.** Regenerating no longer inherits stale frames from a denser prior run (which were fed to the model with fabricated timestamps), and a failed/cancelled run preserves the previous successful frames.
- **Preview matches export.** Text overlays center on their anchor, overlay-effect fades are eased (not linear), blur strength is resolution-independent, and the zoom-pan preview reframes correctly for letterboxed video.
- **Editor paper cuts.** Splitting an image clip no longer duplicates it; `Cmd+S` no longer splits the clip; dragging an effect past the timeline end keeps its duration; the effect inspector no longer floods (and wipes) the undo history; and "+ Effect" near the end no longer silently no-ops.

### Changed
- Text overlay italic/underline/alignment controls were removed (ffmpeg's `drawtext` can't reproduce them); the blur control is now a resolution-independent "Strength". Cached renders re-render once on first open so the text and easing fixes take effect.

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
