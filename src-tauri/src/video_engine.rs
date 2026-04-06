//! Video processing engine using ffmpeg for frame extraction and probing.

use crate::error::NarratorError;
use crate::models::{Frame, FrameConfig, VideoMetadata};
use std::path::{Path, PathBuf};
use tokio::process::Command;

pub fn detect_ffmpeg() -> Result<PathBuf, NarratorError> {
    detect_binary("ffmpeg")
}

pub fn detect_ffprobe() -> Result<PathBuf, NarratorError> {
    detect_binary("ffprobe")
}

/// Detect a bundled sidecar binary (ffmpeg or ffprobe).
/// Checks: next to the app executable (Tauri sidecar), relative paths, then system PATH.
fn detect_binary(name: &str) -> Result<PathBuf, NarratorError> {
    let exe_name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };

    // 1. Next to the current executable (Tauri bundles sidecars here)
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let candidate = exe_dir.join(&exe_name);
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }

    // 2. Common relative sidecar paths (dev mode)
    for dir in ["./binaries", "../binaries"] {
        let candidate = PathBuf::from(dir).join(&exe_name);
        if candidate.exists() {
            return Ok(candidate);
        }
        // Also try without .exe for dev mode on macOS/Linux
        if cfg!(windows) {
            let candidate = PathBuf::from(dir).join(name);
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }

    // 3. System PATH lookup
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(output) = std::process::Command::new("which").arg(name).output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Ok(PathBuf::from(path));
                }
            }
        }
        // macOS common paths
        for p in [
            format!("/usr/local/bin/{name}"),
            format!("/opt/homebrew/bin/{name}"),
        ] {
            if Path::new(&p).exists() {
                return Ok(PathBuf::from(p));
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(output) = std::process::Command::new("where").arg(name).output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !path.is_empty() {
                    return Ok(PathBuf::from(path));
                }
            }
        }
    }

    Err(NarratorError::FfmpegNotFound)
}

pub async fn probe_video(path: &Path) -> Result<VideoMetadata, NarratorError> {
    let ffprobe = detect_ffprobe()?;

    let output = Command::new(ffprobe.as_os_str())
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_streams",
            "-show_format",
        ])
        .arg(path.as_os_str())
        .output()
        .await
        .map_err(|e| NarratorError::VideoProbeError(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(NarratorError::VideoProbeError(stderr.to_string()));
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|e| {
        NarratorError::VideoProbeError(format!("Failed to parse ffprobe output: {e}"))
    })?;

    let video_stream = json["streams"]
        .as_array()
        .and_then(|streams| streams.iter().find(|s| s["codec_type"] == "video"))
        .ok_or_else(|| NarratorError::VideoProbeError("No video stream found".to_string()))?;

    let width = video_stream["width"].as_u64().unwrap_or(0) as u32;
    let height = video_stream["height"].as_u64().unwrap_or(0) as u32;
    let codec = video_stream["codec_name"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    // Parse fps from r_frame_rate (e.g., "30/1" or "30000/1001")
    let fps = parse_frame_rate(video_stream["r_frame_rate"].as_str().unwrap_or("0/1"));

    let duration = json["format"]["duration"]
        .as_str()
        .and_then(|d| d.parse::<f64>().ok())
        .unwrap_or(0.0);

    let file_size = json["format"]["size"]
        .as_str()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    Ok(VideoMetadata {
        path: path.to_string_lossy().to_string(),
        duration_seconds: duration,
        width,
        height,
        codec,
        fps,
        file_size,
    })
}

/// Probe the duration of any media file (audio or video).
pub async fn probe_duration(path: &Path) -> Result<f64, NarratorError> {
    let ffprobe = detect_ffprobe()?;

    let output = Command::new(ffprobe.as_os_str())
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_format",
        ])
        .arg(path.as_os_str())
        .output()
        .await
        .map_err(|e| NarratorError::VideoProbeError(e.to_string()))?;

    if !output.status.success() {
        return Err(NarratorError::VideoProbeError("ffprobe failed".into()));
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| NarratorError::VideoProbeError(e.to_string()))?;

    json["format"]["duration"]
        .as_str()
        .and_then(|d| d.parse::<f64>().ok())
        .ok_or_else(|| NarratorError::VideoProbeError("No duration found".into()))
}

fn parse_frame_rate(rate: &str) -> f64 {
    let parts: Vec<&str> = rate.split('/').collect();
    if parts.len() == 2 {
        let num = parts[0].parse::<f64>().unwrap_or(0.0);
        let den = parts[1].parse::<f64>().unwrap_or(1.0);
        if den > 0.0 {
            return num / den;
        }
    }
    rate.parse::<f64>().unwrap_or(0.0)
}

pub async fn extract_frames(
    video_path: &Path,
    config: &FrameConfig,
    output_dir: &Path,
    on_progress: impl Fn(Frame),
) -> Result<Vec<Frame>, NarratorError> {
    let ffmpeg = detect_ffmpeg()?;

    // Ensure output dir exists
    std::fs::create_dir_all(output_dir)
        .map_err(|e| NarratorError::FrameExtractionError(e.to_string()))?;

    let metadata = probe_video(video_path).await?;
    let interval = config.density.interval_seconds();

    // Extract frames at fixed intervals
    let output_pattern = output_dir.join("frame_%04d.jpg");
    let vf_filter = format!("fps=1/{interval}");

    let output = Command::new(ffmpeg.as_os_str())
        .args([
            "-i",
            &video_path.to_string_lossy(),
            "-vf",
            &vf_filter,
            "-q:v",
            "2",
            "-y",
        ])
        .arg(output_pattern.as_os_str())
        .output()
        .await
        .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(NarratorError::FfmpegFailed(stderr.to_string()));
    }

    // Collect extracted frames
    let mut frames = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(output_dir)
        .map_err(|e| NarratorError::FrameExtractionError(e.to_string()))?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext == "jpg" || ext == "jpeg")
        })
        .collect();

    entries.sort_by_key(|e| e.file_name());

    for (i, entry) in entries.iter().enumerate() {
        if i >= config.max_frames {
            break;
        }

        let path = entry.path();
        let timestamp = i as f64 * interval;

        // Get image dimensions
        let (width, height) = get_image_dimensions(&path).unwrap_or((0, 0));

        let frame = Frame {
            index: i,
            timestamp_seconds: timestamp.min(metadata.duration_seconds),
            path: path.clone(),
            width,
            height,
        };

        on_progress(frame.clone());
        frames.push(frame);
    }

    // Deduplicate similar frames using blake3 hashing
    let frames = deduplicate_frames(frames);

    Ok(frames)
}

fn get_image_dimensions(path: &Path) -> Option<(u32, u32)> {
    image::image_dimensions(path).ok()
}

fn deduplicate_frames(frames: Vec<Frame>) -> Vec<Frame> {
    if frames.len() <= 1 {
        return frames;
    }

    let mut unique_frames = vec![frames[0].clone()];
    let mut prev_hash = hash_frame_file(&frames[0].path);

    for frame in frames.iter().skip(1) {
        let current_hash = hash_frame_file(&frame.path);
        if current_hash != prev_hash {
            unique_frames.push(frame.clone());
            prev_hash = current_hash;
        }
    }

    // Re-index
    for (i, frame) in unique_frames.iter_mut().enumerate() {
        frame.index = i;
    }

    unique_frames
}

fn hash_frame_file(path: &Path) -> String {
    match std::fs::read(path) {
        Ok(data) => {
            // Create a small thumbnail-like hash by using the raw bytes
            let hash = blake3::hash(&data);
            hash.to_hex().to_string()
        }
        Err(_) => String::new(),
    }
}

pub fn frame_to_base64(path: &Path) -> Result<String, NarratorError> {
    let data = std::fs::read(path)?;
    Ok(base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        &data,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frame_rate() {
        assert!((parse_frame_rate("30/1") - 30.0).abs() < 0.01);
        assert!((parse_frame_rate("30000/1001") - 29.97).abs() < 0.01);
        assert!((parse_frame_rate("24/1") - 24.0).abs() < 0.01);
        assert!((parse_frame_rate("0/1") - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_detect_ffmpeg() {
        // This test will pass if ffmpeg is installed
        let result = detect_ffmpeg();
        if result.is_ok() {
            let path = result.unwrap();
            assert!(path.to_string_lossy().contains("ffmpeg"));
        }
    }

    #[test]
    fn test_deduplicate_frames_empty() {
        let frames: Vec<Frame> = vec![];
        let result = deduplicate_frames(frames);
        assert!(result.is_empty());
    }

    #[test]
    fn test_deduplicate_frames_single() {
        let frames = vec![Frame {
            index: 0,
            timestamp_seconds: 0.0,
            path: PathBuf::from("/nonexistent"),
            width: 100,
            height: 100,
        }];
        let result = deduplicate_frames(frames);
        assert_eq!(result.len(), 1);
    }
}
