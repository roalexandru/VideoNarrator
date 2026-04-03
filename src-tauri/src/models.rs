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
}

impl Default for FrameConfig {
    fn default() -> Self {
        Self {
            density: FrameDensity::Medium,
            scene_threshold: 0.3,
            max_frames: 30,
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
}

impl std::fmt::Display for AiProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AiProviderKind::Claude => write!(f, "claude"),
            AiProviderKind::OpenAi => write!(f, "openai"),
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pace {
    Slow,
    Medium,
    Fast,
}

impl Default for Pace {
    fn default() -> Self {
        Pace::Medium
    }
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

// ── Project ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegenerateParams {
    pub frames: Vec<Frame>,
    pub context: String,
    pub segment_index: usize,
    pub style: String,
    pub language: String,
    pub ai_config: AiConfig,
}

// ── Export ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportOptions {
    pub formats: Vec<ExportFormat>,
    pub languages: Vec<String>,
    pub output_directory: String,
    pub scripts: std::collections::HashMap<String, NarrationScript>,
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
