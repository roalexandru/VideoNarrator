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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    pub provider: AiProviderKind,
    pub model: String,
    pub temperature: f32,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: AiProviderKind::Claude,
            model: "claude-sonnet-4-20250514".to_string(),
            temperature: 0.7,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderKeyStatus {
    pub provider: AiProviderKind,
    pub has_key: bool,
    pub models: Vec<String>,
}

// ── Narration Script ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NarrationScript {
    pub title: String,
    pub total_duration_seconds: f64,
    pub segments: Vec<Segment>,
    pub metadata: ScriptMetadata,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub skip_frames: bool,
    pub fps_override: Option<f64>,
    #[serde(default)]
    pub clip_type: Option<String>,
    #[serde(default)]
    pub freeze_source_time: Option<f64>,
    #[serde(default)]
    pub freeze_duration: Option<f64>,
    #[serde(default)]
    pub zoom_pan: Option<ZoomPanEffect>,
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
    #[serde(rename = "progress")]
    Progress { percent: f64 },
    #[serde(rename = "frame_extracted")]
    FrameExtracted { frame: Frame },
    #[serde(rename = "segment_streamed")]
    SegmentStreamed { segment: Segment },
    #[serde(rename = "error")]
    Error { message: String },
}
