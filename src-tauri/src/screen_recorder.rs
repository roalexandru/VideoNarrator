//! Screen recording using native OS tools (macOS screencapture, Windows ffmpeg).
#![allow(dead_code)]

use crate::error::NarratorError;
use crate::video_engine;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingConfig {
    pub output_path: String,
    pub screen_index: u32,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub offset_x: u32,
    pub offset_y: u32,
    pub capture_audio: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenDevice {
    pub index: u32,
    pub name: String,
    pub is_screen: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    pub id: u32,
    pub name: String,
    pub owner: String,
}

/// macOS: Use native `screencapture -v` which opens Apple's Cmd+Shift+5 UI
/// Windows: Use ffmpeg gdigrab
/// Returns the output path when recording finishes
pub async fn record_native(output_path: &str) -> Result<String, NarratorError> {
    if let Some(parent) = PathBuf::from(output_path).parent() {
        std::fs::create_dir_all(parent)?;
    }

    #[cfg(target_os = "macos")]
    {
        tracing::info!("Starting native macOS screen recording to {output_path}");

        // -J video: opens interactive capture in video recording mode (the Cmd+Shift+5 toolbar)
        // -P: opens result in QuickTime Player after (but we intercept the file)
        // This blocks until the user stops recording via the toolbar stop button
        let output = tokio::process::Command::new("screencapture")
            .args(["-J", "video", output_path])
            .output()
            .await
            .map_err(|e| NarratorError::FfmpegFailed(format!("screencapture failed: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Exit code 1 with empty stderr means user cancelled — that's OK
            if stderr.is_empty() {
                return Err(NarratorError::Cancelled);
            }
            return Err(NarratorError::FfmpegFailed(format!("screencapture error: {stderr}")));
        }

        // Check if file was actually created (user might have cancelled)
        if !PathBuf::from(output_path).exists() {
            return Err(NarratorError::Cancelled);
        }

        Ok(output_path.to_string())
    }

    #[cfg(target_os = "windows")]
    {
        // Windows fallback: use ffmpeg gdigrab
        let ffmpeg = video_engine::detect_ffmpeg()?;
        let output = tokio::process::Command::new(ffmpeg.as_os_str())
            .args(["-y", "-f", "gdigrab", "-framerate", "30", "-i", "desktop",
                "-vcodec", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p",
                "-t", "300", // max 5 minutes
                output_path])
            .output()
            .await
            .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(NarratorError::FfmpegFailed(stderr.to_string()));
        }
        Ok(output_path.to_string())
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Err(NarratorError::FfmpegFailed("Screen recording not supported on this platform".into()))
    }
}

/// Legacy ffmpeg-based recording with stop flag (for Windows)
pub async fn start_recording(
    config: &RecordingConfig,
    stop_flag: Arc<AtomicBool>,
) -> Result<String, NarratorError> {
    let ffmpeg = video_engine::detect_ffmpeg()?;
    let output_path = &config.output_path;

    if let Some(parent) = PathBuf::from(output_path).parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut cmd = tokio::process::Command::new(ffmpeg.as_os_str());

    #[cfg(target_os = "windows")]
    {
        cmd.args(["-y", "-f", "gdigrab", "-framerate", &config.fps.to_string()]);
        if config.width > 0 && config.height > 0 {
            cmd.args(["-offset_x", &config.offset_x.to_string()]);
            cmd.args(["-offset_y", &config.offset_y.to_string()]);
            cmd.args(["-video_size", &format!("{}x{}", config.width, config.height)]);
        }
        cmd.args(["-i", "desktop"]);
    }

    #[cfg(target_os = "macos")]
    {
        let input = format!("{}:none", config.screen_index);
        cmd.args(["-y", "-f", "avfoundation", "-capture_cursor", "1",
            "-framerate", &config.fps.to_string(), "-i", &input]);
        if config.width > 0 && config.height > 0 {
            cmd.args(["-vf", &format!("crop={}:{}:{}:{}", config.width, config.height, config.offset_x, config.offset_y)]);
        }
    }

    cmd.args(["-vcodec", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"]);
    cmd.arg(output_path);

    let mut child = cmd.stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| NarratorError::FfmpegFailed(format!("Failed to start recording: {e}")))?;

    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if stop_flag.load(Ordering::SeqCst) {
                if let Some(stdin) = child.stdin.as_mut() {
                    use tokio::io::AsyncWriteExt;
                    let _ = stdin.write_all(b"q").await;
                }
                let _ = child.wait().await;
                break;
            }
        }
    });

    Ok(output_path.to_string())
}
