//! Error types for the Narrator application.

use serde::Serialize;

#[allow(dead_code)]
#[derive(Debug, thiserror::Error)]
pub enum NarratorError {
    #[error("ffmpeg not found. Install ffmpeg or configure the sidecar path.")]
    FfmpegNotFound,

    #[error("ffmpeg failed: {0}")]
    FfmpegFailed(String),

    #[error("Video probe error: {0}")]
    VideoProbeError(String),

    #[error("Frame extraction error: {0}")]
    FrameExtractionError(String),

    #[error("Document processing error: {0}")]
    DocumentError(String),

    #[error("AI API error: {0}")]
    ApiError(String),

    #[error("Rate limited by API provider. Please wait and try again.")]
    RateLimited,

    #[error("Project error: {0}")]
    ProjectError(String),

    #[error("Export error: {0}")]
    ExportError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Operation cancelled")]
    Cancelled,

    #[error("No API key configured for provider: {0}")]
    NoApiKey(String),

    #[error("Authentication error: {0}")]
    AuthError(String),

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("Invalid response: {0}")]
    InvalidResponse(String),
}

impl Serialize for NarratorError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl From<serde_json::Error> for NarratorError {
    fn from(e: serde_json::Error) -> Self {
        NarratorError::SerializationError(e.to_string())
    }
}

impl From<reqwest::Error> for NarratorError {
    fn from(e: reqwest::Error) -> Self {
        NarratorError::ApiError(e.to_string())
    }
}
