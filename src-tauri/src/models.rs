//! Data models and types shared between frontend and backend.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ── Video ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoMetadata {
    pub path: String,
    pub duration_seconds: f64,
    pub width: u32,
    pub height: u32,
    pub codec: String,
    pub fps: f64,
    pub file_size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frame {
    pub index: usize,
    pub timestamp_seconds: f64,
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameConfig {
    pub density: FrameDensity,
    pub scene_threshold: f64,
    pub max_frames: usize,
    #[serde(default)]
    pub skip_dedup: bool,
}

impl Default for FrameConfig {
    fn default() -> Self {
        Self {
            density: FrameDensity::Medium,
            scene_threshold: 0.3,
            max_frames: 30,
            skip_dedup: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FrameDensity {
    Light,
    Medium,
    Heavy,
}

impl FrameDensity {
    pub fn interval_seconds(&self) -> f64 {
        match self {
            FrameDensity::Light => 10.0,
            FrameDensity::Medium => 5.0,
            FrameDensity::Heavy => 2.0,
        }
    }
}

// ── Documents ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessedDocument {
    pub name: String,
    pub content: String,
    pub token_estimate: usize,
    pub source_path: String,
}

// ── AI Provider ──

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum AiProviderKind {
    Claude,
    #[serde(rename = "openai")]
    OpenAi,
    #[serde(rename = "gemini")]
    Gemini,
}

impl std::fmt::Display for AiProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AiProviderKind::Claude => write!(f, "claude"),
            AiProviderKind::OpenAi => write!(f, "openai"),
            AiProviderKind::Gemini => write!(f, "gemini"),
        }
    }
}

/// Provider-agnostic reasoning depth.
///
/// Every current frontier model exposes a "how hard should you think" knob, but
/// each vendor spells it differently — Anthropic `output_config.effort`, OpenAI
/// `reasoning.effort`, Google `generationConfig.thinkingLevel`. We expose one
/// product-level choice and map it per provider in `ai_client`, clamping to what
/// each model actually accepts (Gemini, for example, tops out at `high`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    /// Least thinking: fastest and cheapest. Good for short, simple videos.
    Fast,
    /// Default — the cost/quality sweet spot for most narration.
    #[default]
    Balanced,
    /// More deliberate analysis of the frames before writing.
    Thorough,
    /// Maximum reasoning depth. Slowest and most expensive.
    Max,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    pub provider: AiProviderKind,
    pub model: String,
    pub temperature: f32,
    /// Absent in projects saved before reasoning selection existed — those load
    /// as `Balanced`.
    #[serde(default)]
    pub reasoning_effort: ReasoningEffort,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: AiProviderKind::Claude,
            // Successor to the previous default (`claude-sonnet-4-20250514`) per
            // Anthropic's model rename table — same tier, current generation.
            model: "claude-sonnet-5".to_string(),
            temperature: 0.7,
            reasoning_effort: ReasoningEffort::Balanced,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderKeyStatus {
    pub provider: AiProviderKind,
    pub has_key: bool,
    pub models: Vec<String>,
}

// ── Render quality ──

/// How much encode time to spend on a render.
///
/// Every render was previously final-quality, so checking whether a zoom looked
/// right cost the same as producing the deliverable. A preview tier makes the
/// iterate-on-edits loop cheap; the final tier is unchanged, so nothing the user
/// ships is affected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RenderQuality {
    /// Visually lossless and universally playable. What gets delivered.
    #[default]
    Final,
    /// Fast enough to iterate on, good enough to judge framing and timing.
    Preview,
    /// Fastest possible — for confirming a change landed at all.
    Draft,
}

impl RenderQuality {
    /// libx264 `-crf`. Higher is smaller and lossier.
    pub fn crf(self) -> &'static str {
        match self {
            // Matches the previous hard-coded value; see `encoder.rs` for why
            // not CRF 0 (huge files, undecodable on some consumer players).
            RenderQuality::Final => "18",
            RenderQuality::Preview => "24",
            RenderQuality::Draft => "30",
        }
    }

    /// libx264 `-preset`. Faster presets trade compression for speed.
    pub fn preset(self) -> &'static str {
        match self {
            RenderQuality::Final => "medium",
            RenderQuality::Preview => "veryfast",
            RenderQuality::Draft => "ultrafast",
        }
    }

    /// Height ceiling, or `None` to keep the source resolution.
    ///
    /// Downscaling is what makes a preview genuinely fast on a 4K source; the
    /// final tier never downscales, because that would silently degrade the
    /// deliverable.
    pub fn max_height(self) -> Option<u32> {
        match self {
            RenderQuality::Final => None,
            RenderQuality::Preview => Some(720),
            RenderQuality::Draft => Some(480),
        }
    }
}

// ── Narration Script ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NarrationScript {
    pub title: String,
    pub total_duration_seconds: f64,
    pub segments: Vec<Segment>,
    pub metadata: ScriptMetadata,
    /// Per-segment prediction of whether the text will fit inside its window
    /// at natural TTS speed. Populated by `script_validator::validate_speech_rate`
    /// at generation time and consumed by the Review UI. Serialized so it
    /// persists to disk and the frontend doesn't need to recompute on load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speech_rate_report: Option<Vec<crate::speech_rate::SegmentOverflow>>,
}

/// A stretch of the source audio quiet enough to place narration over.
///
/// Produced by `video_engine`'s `silencedetect` pass. The pass already ran to
/// pick frame anchors; keeping the spans instead of collapsing them to
/// midpoints lets the narration timeline avoid talking over existing audio.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SilenceSpan {
    pub start: f64,
    pub end: f64,
}

impl SilenceSpan {
    pub fn duration(&self) -> f64 {
        (self.end - self.start).max(0.0)
    }

    pub fn midpoint(&self) -> f64 {
        0.5 * (self.start + self.end)
    }

    /// True when `t` falls inside the span.
    pub fn contains(&self, t: f64) -> bool {
        t >= self.start && t <= self.end
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    #[serde(default)]
    pub index: usize,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
    #[serde(default)]
    pub visual_description: String,
    #[serde(default)]
    pub emphasis: Vec<String>,
    #[serde(default = "Pace::default")]
    pub pace: Pace,
    #[serde(default)]
    pub pause_after_ms: u32,
    #[serde(default)]
    pub frame_refs: Vec<usize>,
    /// Per-segment voice override. None = use project default voice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_override: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum Pace {
    Slow,
    #[default]
    Medium,
    Fast,
}

impl std::fmt::Display for Pace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Pace::Slow => write!(f, "slow"),
            Pace::Medium => write!(f, "medium"),
            Pace::Fast => write!(f, "fast"),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScriptMetadata {
    #[serde(default)]
    pub style: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub generated_at: String,
}

// ── Narration Style ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NarrationStyle {
    pub id: String,
    pub label: String,
    pub description: String,
    pub system_prompt: String,
    pub pacing: String,
    pub pause_markers: bool,
}

fn default_schema_version() -> u32 {
    1
}

// ── Zoom/Pan ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoomRegion {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EasingPreset {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZoomPanEffect {
    pub start_region: ZoomRegion,
    pub end_region: ZoomRegion,
    pub easing: EasingPreset,
}

// ── Project ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditClip {
    pub source_start: f64,
    pub source_end: f64,
    pub speed: f64,
    #[serde(default)]
    pub skip_frames: bool,
    pub fps_override: Option<f64>,
    #[serde(default)]
    pub clip_type: Option<String>,
    #[serde(default)]
    pub freeze_source_time: Option<f64>,
    #[serde(default)]
    pub freeze_duration: Option<f64>,
    /// Which MediaRef this clip sources from. `None` on legacy projects and
    /// on clips that point at the project's primary video; frontend resolves
    /// absent → primary.
    #[serde(default)]
    pub media_ref_id: Option<String>,
    /// Output-timeline duration for `clip_type == "image"` clips.
    #[serde(default)]
    pub image_duration: Option<f64>,
    #[serde(default)]
    pub zoom_pan: Option<ZoomPanEffect>,
}

/// Extra source file added via the timeline "+" button (mixes in with the
/// project's primary video). Keyed by id inside `ProjectConfig.media_pool`;
/// each clip references one of these via `EditClip.media_ref_id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMediaRef {
    pub hash: String,
    pub kind: String,
    pub path: String,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub fps: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub id: String,
    pub title: String,
    pub description: String,
    pub video_path: String,
    pub style: String,
    pub languages: Vec<String>,
    pub primary_language: String,
    pub frame_config: FrameConfig,
    pub ai_config: AiConfig,
    pub custom_prompt: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub edit_clips: Option<Vec<EditClip>>,
    /// Additional source files imported via the timeline "+" button. Keyed
    /// by MediaRef id (e.g. a UUID). The primary video is NOT stored here —
    /// it's derived from `video_metadata`. Absent on legacy projects.
    #[serde(default)]
    pub media_pool: Option<std::collections::HashMap<String, ProjectMediaRef>>,
    #[serde(default)]
    pub timeline_effects: Option<serde_json::Value>,
    #[serde(default)]
    pub video_metadata: Option<VideoMetadata>,
    /// Persisted context documents (PDF/MD/TXT paths + metadata) so the AI
    /// narration prompt can be regenerated with the same inputs after load.
    #[serde(default)]
    pub context_documents: Option<serde_json::Value>,
    /// Absolute path to the cached edited video (produced by apply_video_edits).
    /// Exporting uses this file so the final render includes all clip + effect
    /// edits. Invalidated by a hash mismatch against edit_clips + timeline_effects.
    #[serde(default)]
    pub edited_video_path: Option<String>,
    /// Hash of the edit_clips + timeline_effects used to produce
    /// `edited_video_path`. If the current edits hash differently, the cached
    /// video is stale and Export will regenerate it.
    #[serde(default)]
    pub edited_video_plan_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub id: String,
    pub title: String,
    pub video_path: String,
    pub style: String,
    pub created_at: String,
    pub updated_at: String,
    pub has_script: bool,
    pub thumbnail_path: Option<String>,
    pub script_languages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadedProject {
    pub config: ProjectConfig,
    pub scripts: std::collections::HashMap<String, NarrationScript>,
}

// ── Templates ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectTemplate {
    pub id: String,
    pub name: String,
    pub style: String,
    pub languages: Vec<String>,
    pub primary_language: String,
    pub frame_config: FrameConfig,
    pub ai_config: AiConfig,
    pub custom_prompt: String,
    #[serde(default)]
    pub tts_provider: String,
    #[serde(default)]
    pub created_at: String,
}

// ── Generation ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationParams {
    #[serde(default)]
    pub project_id: String,
    pub video_path: String,
    pub document_paths: Vec<String>,
    pub title: String,
    pub description: String,
    pub style: String,
    pub primary_language: String,
    pub additional_languages: Vec<String>,
    pub frame_config: FrameConfig,
    pub ai_config: AiConfig,
    pub custom_prompt: String,
    /// Segments from a prior partial run. When present, `generate_chunked`
    /// seeds its accumulator with these and skips chunks whose frames are
    /// entirely before the last segment's `end_seconds`, so API calls that
    /// already succeeded are not re-billed on retry.
    #[serde(default)]
    pub resume_segments: Vec<Segment>,
    /// When true, run a self-critique pass after the main generation: the
    /// model re-reads the draft against sampled frames and suggests fixes
    /// for segments whose narration contradicts the visible content.
    /// Disabled by default — it adds one extra multimodal API call plus up
    /// to five text-only refine calls per iteration.
    #[serde(default)]
    pub strict_mode: bool,
    /// Send frames as tiled contact sheets instead of one image per frame.
    ///
    /// Nine moments then occupy one image slot, so a long video needs a handful
    /// of API calls rather than thirty — and each call keeps full context
    /// instead of a truncated summary of the previous one.
    ///
    /// **Deliberately still off by default**, unlike the other two flags below.
    ///
    /// Tiling halves each frame's linear resolution (1024 px → 512 px cells), and
    /// reading on-screen text is exactly what this app asks the model to do. OCR
    /// (`use_screen_text`) compensates for that loss — but only where a
    /// recognizer exists, which today means macOS only. Enabling tiling globally
    /// would therefore make Windows output measurably worse than macOS with
    /// nothing offsetting it: a silent cross-platform quality split.
    ///
    /// The upside here is cost and latency, not quality. That makes it the one
    /// flag whose benefit is purely economic while its risk is to the output, so
    /// it stays opt-in until the A/B on a terminal-heavy screencast settles the
    /// cell size (the fallback being 2x2 at 768 px).
    #[serde(default)]
    pub use_contact_sheets: bool,

    /// Let the model choose which moments to extract at full resolution.
    ///
    /// Replaces the even-spaced subsample in `merge_anchors` with a cheap survey
    /// pass: one low-resolution decode covering the whole video, tiled into
    /// contact sheets, and one structured call asking which timestamps carry
    /// meaningful change. Only the chosen moments then pay the expensive
    /// frame-accurate extraction.
    ///
    /// On by default. It costs one cheap low-resolution call before narration
    /// starts, and every failure path — survey extraction, the call itself,
    /// unparseable output, an empty selection — falls back to the previous
    /// even-spaced selection. So the downside is bounded at one wasted request
    /// while the upside is the model spending its frame budget on the moments
    /// that actually change.
    #[serde(default = "default_true")]
    pub use_model_frame_selection: bool,
    /// Read the text visible on screen and give it to the model.
    ///
    /// The system prompt already asks the model to read terminals and code
    /// editors, but a 1024 px downscaled JPEG often cannot deliver that. OCR over
    /// the full-resolution frames makes it reliable — the difference between
    /// "runs a command" and "runs `pnpm tauri dev`".
    ///
    /// On by default. The OCR pass is local and parallel across frames, the
    /// prompt cost is bounded at ~10 KB, and where no recognizer exists (today:
    /// anything but macOS) the flag is inert rather than broken — the pipeline
    /// runs, produces an empty pack, and generation proceeds as before.
    #[serde(default = "default_true")]
    pub use_screen_text: bool,
}

/// Serde default for flags that ship enabled.
///
/// Needed because `bool`'s own `Default` is `false`, so an absent field in a
/// project saved before the flag existed would silently opt out of it.
fn default_true() -> bool {
    true
}

// ── Export ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportOptions {
    pub formats: Vec<ExportFormat>,
    pub languages: Vec<String>,
    pub output_directory: String,
    pub scripts: std::collections::HashMap<String, NarrationScript>,
    #[serde(default)]
    pub basename: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Json,
    Srt,
    Vtt,
    Txt,
    #[serde(rename = "md")]
    Markdown,
    Ssml,
}

impl std::fmt::Display for ExportFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportFormat::Json => write!(f, "json"),
            ExportFormat::Srt => write!(f, "srt"),
            ExportFormat::Vtt => write!(f, "vtt"),
            ExportFormat::Txt => write!(f, "txt"),
            ExportFormat::Markdown => write!(f, "md"),
            ExportFormat::Ssml => write!(f, "ssml"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportResult {
    pub format: String,
    pub language: String,
    pub file_path: String,
    pub success: bool,
    pub error: Option<String>,
}

// ── Project Frames (for timeline thumbnails) ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectFrame {
    pub index: usize,
    pub path: String,
}

// ── Progress Events ──

#[cfg(test)]
mod tests {
    use super::*;

    // ── Feature-flag defaults ───────────────────────────────────────────────
    //
    // These are the contract for what a caller gets when it says nothing. A
    // project saved before a flag existed, the CLI, and any external caller all
    // land here, so each default is asserted individually rather than trusted.

    /// Minimal params JSON with none of the feature flags present — exactly what
    /// a project saved before they existed deserializes from.
    fn params_without_flags() -> serde_json::Value {
        serde_json::json!({
            "project_id": "11111111-1111-4111-8111-111111111111",
            "video_path": "/tmp/v.mp4",
            "document_paths": [],
            "title": "T",
            "description": "D",
            "style": "technical",
            "primary_language": "en",
            "additional_languages": [],
            "frame_config": {"density": "medium", "scene_threshold": 0.3, "max_frames": 30},
            "ai_config": {"provider": "claude", "model": "claude-sonnet-5", "temperature": 0.7},
            "custom_prompt": ""
        })
    }

    #[test]
    fn screen_text_and_frame_selection_default_on() {
        let params: GenerationParams = serde_json::from_value(params_without_flags())
            .expect("legacy params must still deserialize");
        assert!(
            params.use_screen_text,
            "OCR grounding must be on for callers that don't mention it"
        );
        assert!(
            params.use_model_frame_selection,
            "model frame selection must be on for callers that don't mention it"
        );
    }

    #[test]
    fn contact_sheets_default_off() {
        // Deliberate: tiling halves the resolution the model sees, and OCR only
        // compensates on platforms with a recognizer. Enabling it globally would
        // make Windows output worse than macOS with nothing offsetting it.
        let params: GenerationParams = serde_json::from_value(params_without_flags()).unwrap();
        assert!(!params.use_contact_sheets);
    }

    #[test]
    fn strict_mode_still_defaults_off() {
        // Unchanged by this release — it costs up to 12 extra API calls.
        let params: GenerationParams = serde_json::from_value(params_without_flags()).unwrap();
        assert!(!params.strict_mode);
    }

    #[test]
    fn an_explicit_false_still_wins_over_the_default() {
        // `default = "default_true"` must not override a caller that deliberately
        // opted out, or the toggles in the UI would do nothing.
        let mut raw = params_without_flags();
        raw["use_screen_text"] = serde_json::json!(false);
        raw["use_model_frame_selection"] = serde_json::json!(false);
        let params: GenerationParams = serde_json::from_value(raw).unwrap();
        assert!(!params.use_screen_text);
        assert!(!params.use_model_frame_selection);
    }

    #[test]
    fn an_explicit_true_enables_contact_sheets() {
        let mut raw = params_without_flags();
        raw["use_contact_sheets"] = serde_json::json!(true);
        let params: GenerationParams = serde_json::from_value(raw).unwrap();
        assert!(params.use_contact_sheets);
    }

    #[test]
    fn default_true_helper_is_actually_true() {
        // Guards against the classic typo of wiring `default = "default_true"`
        // to a function that returns false.
        assert!(default_true());
    }

    #[test]
    fn test_serialize_narration_script() {
        let script = NarrationScript {
            title: "Roundtrip Test".to_string(),
            total_duration_seconds: 60.0,
            segments: vec![Segment {
                index: 0,
                start_seconds: 0.0,
                end_seconds: 30.0,
                text: "Hello world.".to_string(),
                visual_description: "Opening scene".to_string(),
                emphasis: vec!["world".to_string()],
                pace: Pace::Slow,
                pause_after_ms: 200,
                frame_refs: vec![0, 1],
                voice_override: None,
            }],
            metadata: ScriptMetadata {
                style: "technical".to_string(),
                language: "en".to_string(),
                provider: "claude".to_string(),
                model: "test-model".to_string(),
                generated_at: "2026-01-01T00:00:00Z".to_string(),
            },
            speech_rate_report: None,
        };

        // Serialize to JSON
        let json = serde_json::to_string(&script).unwrap();

        // Deserialize back
        let deserialized: NarrationScript = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.title, "Roundtrip Test");
        assert_eq!(deserialized.total_duration_seconds, 60.0);
        assert_eq!(deserialized.segments.len(), 1);
        assert_eq!(deserialized.segments[0].text, "Hello world.");
        assert_eq!(deserialized.segments[0].emphasis, vec!["world".to_string()]);
        assert_eq!(deserialized.metadata.style, "technical");
        assert_eq!(deserialized.metadata.language, "en");
        assert_eq!(deserialized.metadata.model, "test-model");
    }

    #[test]
    fn test_frame_density_intervals() {
        assert!((FrameDensity::Light.interval_seconds() - 10.0).abs() < 0.01);
        assert!((FrameDensity::Medium.interval_seconds() - 5.0).abs() < 0.01);
        assert!((FrameDensity::Heavy.interval_seconds() - 2.0).abs() < 0.01);
    }

    #[test]
    fn test_ai_provider_kind_display() {
        assert_eq!(AiProviderKind::Claude.to_string(), "claude");
        assert_eq!(AiProviderKind::OpenAi.to_string(), "openai");
        assert_eq!(AiProviderKind::Gemini.to_string(), "gemini");
    }

    #[test]
    fn test_export_format_display() {
        assert_eq!(ExportFormat::Json.to_string(), "json");
        assert_eq!(ExportFormat::Srt.to_string(), "srt");
        assert_eq!(ExportFormat::Vtt.to_string(), "vtt");
        assert_eq!(ExportFormat::Txt.to_string(), "txt");
        assert_eq!(ExportFormat::Markdown.to_string(), "md");
        assert_eq!(ExportFormat::Ssml.to_string(), "ssml");
    }

    #[test]
    fn test_pace_display() {
        assert_eq!(Pace::Slow.to_string(), "slow");
        assert_eq!(Pace::Medium.to_string(), "medium");
        assert_eq!(Pace::Fast.to_string(), "fast");
    }

    #[test]
    fn test_pace_default() {
        let pace = Pace::default();
        assert_eq!(pace.to_string(), "medium");
    }

    #[test]
    fn test_frame_config_default() {
        let config = FrameConfig::default();
        assert!((config.density.interval_seconds() - 5.0).abs() < 0.01);
        assert!((config.scene_threshold - 0.3).abs() < 0.01);
        assert_eq!(config.max_frames, 30);
    }

    #[test]
    fn test_ai_config_default() {
        let config = AiConfig::default();
        assert_eq!(config.provider.to_string(), "claude");
        assert!(config.model.contains("sonnet"));
        assert!((config.temperature - 0.7).abs() < 0.01);
    }

    #[test]
    fn test_ai_provider_kind_serde_roundtrip() {
        // Verify serde rename_all = "lowercase" works correctly
        let json = serde_json::to_string(&AiProviderKind::Claude).unwrap();
        assert_eq!(json, "\"claude\"");

        let json = serde_json::to_string(&AiProviderKind::OpenAi).unwrap();
        assert_eq!(json, "\"openai\"");

        let json = serde_json::to_string(&AiProviderKind::Gemini).unwrap();
        assert_eq!(json, "\"gemini\"");

        // Deserialize back
        let provider: AiProviderKind = serde_json::from_str("\"claude\"").unwrap();
        assert_eq!(provider, AiProviderKind::Claude);

        let provider: AiProviderKind = serde_json::from_str("\"openai\"").unwrap();
        assert_eq!(provider, AiProviderKind::OpenAi);

        let provider: AiProviderKind = serde_json::from_str("\"gemini\"").unwrap();
        assert_eq!(provider, AiProviderKind::Gemini);
    }

    #[test]
    fn test_export_format_serde_roundtrip() {
        let json = serde_json::to_string(&ExportFormat::Markdown).unwrap();
        assert_eq!(json, "\"md\"");

        let format: ExportFormat = serde_json::from_str("\"md\"").unwrap();
        assert_eq!(format.to_string(), "md");
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum ProgressEvent {
    #[serde(rename = "phase_change")]
    PhaseChange { phase: String },
    /// Monotonic progress update. `percent` is 0..100 on the emitter's own
    /// domain (the frontend weights/rescales to a global percent). `message`
    /// is an optional human-readable sub-label for *what* is happening right
    /// now ("Processing clip 2 of 5", "Analyzing batch 3 of 4") and is
    /// omitted for intra-stage ticks that would only repeat the same label.
    #[serde(rename = "progress")]
    Progress {
        percent: f64,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        message: Option<String>,
    },
    #[serde(rename = "frame_extracted")]
    FrameExtracted { frame: Frame },
    #[serde(rename = "segment_streamed")]
    SegmentStreamed { segment: Segment },
    /// Emitted once at the end of generation with the full, normalized script.
    /// The frontend replaces its streaming-segments preview with this list so
    /// users see the polished output after chunked generation's raw per-chunk
    /// stream.
    /// Emitted after extracted frames are promoted out of the temp work
    /// directory into the project.
    ///
    /// `FrameExtracted` necessarily carries the *temp* path — it fires while
    /// extraction is still running. That directory is then renamed away, so the
    /// UI's thumbnails 404 and render blank. This replaces the list with the
    /// final, durable paths.
    #[serde(rename = "frames_replaced")]
    FramesReplaced { frames: Vec<Frame> },
    /// What grounding was actually applied to this generation.
    ///
    /// Both features are silent when they no-op (no OCR backend, survey
    /// fell back), which makes "is this even on?" unanswerable from the UI.
    #[serde(rename = "grounding")]
    Grounding {
        /// Distinct screens of recognized text, when OCR ran and found any.
        #[serde(skip_serializing_if = "Option::is_none")]
        screen_text_screens: Option<usize>,
        /// Moments the model chose, when the survey pass succeeded.
        #[serde(skip_serializing_if = "Option::is_none")]
        model_selected_moments: Option<usize>,
    },
    #[serde(rename = "segments_replaced")]
    SegmentsReplaced { segments: Vec<Segment> },
    /// Emitted once after an export finishes, reporting whether the rendered
    /// file matches what was planned. Advisory — the export has already
    /// succeeded by the time this arrives, so a failing check is information,
    /// not an error.
    #[serde(rename = "export_verified")]
    ExportVerified {
        report: crate::export_verify::VerificationReport,
    },
    #[serde(rename = "error")]
    Error { message: String },
}

impl ProgressEvent {
    /// Build a `Progress` event with no message. Use for intra-stage ticks
    /// where a new sub-label would only repeat itself.
    pub fn progress(percent: f64) -> Self {
        ProgressEvent::Progress {
            percent,
            message: None,
        }
    }

    /// Build a `Progress` event carrying a sub-label. Use at milestones
    /// ("Processing clip N of M", "Analyzing batch N of M").
    pub fn progress_msg(percent: f64, message: impl Into<String>) -> Self {
        ProgressEvent::Progress {
            percent,
            message: Some(message.into()),
        }
    }
}
