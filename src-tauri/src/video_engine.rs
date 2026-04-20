//! Video processing engine using ffmpeg for frame extraction and probing.

use crate::error::NarratorError;
use crate::ffmpeg_progress::{extract_time_from_ffmpeg_line, parse_ffmpeg_time};
use crate::models::{Frame, FrameConfig, VideoMetadata};
use crate::process_utils::CommandNoWindow;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
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
        if let Ok(output) = std::process::Command::new("where")
            .no_window()
            .arg(name)
            .output()
        {
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
        .no_window()
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

    // Prefer avg_frame_rate for VFR videos, fall back to r_frame_rate
    let fps_str = video_stream["avg_frame_rate"]
        .as_str()
        .filter(|s| *s != "0/0")
        .or_else(|| video_stream["r_frame_rate"].as_str())
        .unwrap_or("0/1");
    let fps = parse_frame_rate(fps_str);

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
        .no_window()
        .args(["-v", "quiet", "-print_format", "json", "-show_format"])
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
        if den > 0.0 && num >= 0.0 {
            let fps = num / den;
            if fps > 0.0 && fps < 1000.0 {
                return fps;
            }
        }
    }
    let parsed = rate.parse::<f64>().unwrap_or(0.0);
    if parsed > 0.0 && parsed < 1000.0 {
        parsed
    } else {
        30.0 // Safe default for unreadable frame rates
    }
}

/// Extract sampled frames from `video_path` into `output_dir`.
///
/// Two callbacks are invoked during extraction so the UI can show live
/// progress without waiting for ffmpeg to finish:
///
/// - `on_frame(Frame)` — fires once per kept frame (after dedupe + dimension
///   read) so the filmstrip can paint each thumbnail as it's discovered.
/// - `on_tick(fraction, message)` — fires repeatedly with `fraction` ∈ 0..=1
///   across two sub-phases:
///     * `0.0..=0.80` → ffmpeg decoding progress parsed from stderr
///       (`-progress pipe:2` with `-nostats` forces line-terminated output).
///     * `0.80..=1.00` → dimension read / dedup pass in the blocking pool.
///
/// Both callbacks must be `Send + Sync + 'static` because they cross task
/// boundaries (spawn_blocking for dimensions, ffmpeg stderr reader task).
pub async fn extract_frames(
    video_path: &Path,
    config: &FrameConfig,
    output_dir: &Path,
    on_frame: impl Fn(Frame) + Send + Sync + 'static,
    on_tick: impl Fn(f64, String) + Send + Sync + 'static,
) -> Result<Vec<Frame>, NarratorError> {
    let ffmpeg = detect_ffmpeg()?;

    // Ensure output dir exists
    tokio::fs::create_dir_all(output_dir)
        .await
        .map_err(|e| NarratorError::FrameExtractionError(e.to_string()))?;

    let metadata = probe_video(video_path).await?;
    let base_interval = config.density.interval_seconds();

    // Adaptive: ensure we don't extract more frames than max_frames
    // by increasing the interval if needed
    let estimated_frames = (metadata.duration_seconds / base_interval).ceil() as usize;
    let interval = if estimated_frames > config.max_frames && config.max_frames > 0 {
        metadata.duration_seconds / config.max_frames as f64
    } else {
        base_interval
    };
    let expected_frames = if interval > 0.0 {
        ((metadata.duration_seconds / interval).ceil() as usize).min(config.max_frames.max(1))
    } else {
        config.max_frames.max(1)
    };

    // Share the tick callback between the ffmpeg stderr reader and the
    // spawn_blocking dimension pass without leaking its 'static bound into
    // the outer signature twice.
    let on_tick: Arc<dyn Fn(f64, String) + Send + Sync> = Arc::new(on_tick);

    on_tick(0.0, "Starting frame extraction".to_string());

    // Extract frames at fixed intervals. Use `-progress pipe:2` + `-nostats`
    // so stderr is \n-terminated structured progress we can parse line-by-line
    // (see ffmpeg_progress::extract_time_from_ffmpeg_line).
    let output_pattern = output_dir.join("frame_%04d.jpg");
    let vf_filter = format!("fps=1/{interval}");

    let mut child = Command::new(ffmpeg.as_os_str())
        .no_window()
        .args([
            "-progress",
            "pipe:2",
            "-nostats",
            "-i",
            &video_path.to_string_lossy(),
            "-vf",
            &vf_filter,
            "-q:v",
            "2",
            "-y",
        ])
        .arg(output_pattern.as_os_str())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;

    // Tail stderr for `out_time=` and translate each tick into the 0..0.80
    // sub-band. Short-lived extraction runs (<1s) may produce 0 ticks, so the
    // post-ffmpeg passes always emit at least one progress update to move the
    // UI forward even in that edge case.
    const STDERR_TAIL: usize = 40;
    let mut recent_stderr: std::collections::VecDeque<String> =
        std::collections::VecDeque::with_capacity(STDERR_TAIL + 1);
    if let Some(stderr) = child.stderr.take() {
        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();
        let total_duration = metadata.duration_seconds.max(0.001);
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(time_str) = extract_time_from_ffmpeg_line(&line) {
                let seconds = parse_ffmpeg_time(&time_str);
                if seconds > 0.0 {
                    let raw = (seconds / total_duration).clamp(0.0, 1.0);
                    let fraction = raw * 0.80;
                    on_tick(fraction, format!("Extracting frames ({:.0}%)", raw * 100.0));
                }
            }
            if recent_stderr.len() >= STDERR_TAIL {
                recent_stderr.pop_front();
            }
            recent_stderr.push_back(line);
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;
    if !status.success() {
        let tail: String = recent_stderr.iter().cloned().collect::<Vec<_>>().join("\n");
        return Err(NarratorError::FfmpegFailed(tail));
    }

    on_tick(0.80, format!("Indexing frames (0 of ~{expected_frames})"));

    // Collect extracted frames — directory scan, image dimension reads, and blake3
    // hashing are CPU/IO-intensive, so run on the blocking thread pool.
    let output_dir_owned = output_dir.to_path_buf();
    let max_frames = config.max_frames;
    let skip_dedup = config.skip_dedup;
    let duration = metadata.duration_seconds;
    let tick_for_blocking = on_tick.clone();
    let frames = tokio::task::spawn_blocking(move || {
        let mut entries: Vec<_> = std::fs::read_dir(&output_dir_owned)
            .map_err(|e| NarratorError::FrameExtractionError(e.to_string()))?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .is_some_and(|ext| ext == "jpg" || ext == "jpeg")
            })
            .collect();

        entries.sort_by_key(|e| e.file_name());

        let total = entries.len().min(max_frames).max(1);
        let mut frames = Vec::new();
        for (i, entry) in entries.iter().enumerate() {
            if i >= max_frames {
                break;
            }

            let path = entry.path();
            let timestamp = i as f64 * interval;

            let Some((width, height)) = get_image_dimensions(&path) else {
                tracing::warn!(
                    "Skipping frame with unreadable dimensions: {}",
                    path.display()
                );
                continue;
            };

            frames.push(Frame {
                index: i,
                timestamp_seconds: timestamp.min(duration),
                path,
                width,
                height,
            });

            // 0.80..0.95 for dimension reads. Save the final 0.05 for dedupe.
            let fraction = 0.80 + ((i + 1) as f64 / total as f64) * 0.15;
            tick_for_blocking(fraction, format!("Reading frame {} of {}", i + 1, total));
        }

        // Deduplicate similar frames using blake3 hashing (unless skip_dedup is set)
        if skip_dedup {
            Ok::<_, NarratorError>(frames)
        } else {
            let count_before = frames.len();
            let deduped = deduplicate_frames(frames);
            tick_for_blocking(
                0.98,
                format!(
                    "Deduplicating ({} → {} frames)",
                    count_before,
                    deduped.len()
                ),
            );
            Ok::<_, NarratorError>(deduped)
        }
    })
    .await
    .map_err(|e| NarratorError::FrameExtractionError(e.to_string()))??;

    // Report each kept frame back so the filmstrip can paint thumbnails live.
    for frame in &frames {
        on_frame(frame.clone());
    }

    on_tick(1.0, format!("Extracted {} frames", frames.len()));

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

/// Encode a frame as base64 JPEG, downscaling to max_width if larger.
/// Keeps text readable for screen recordings (1024px default).
pub fn frame_to_base64(path: &Path) -> Result<String, NarratorError> {
    frame_to_base64_scaled(path, 1024)
}

pub fn frame_to_base64_scaled(path: &Path, max_width: u32) -> Result<String, NarratorError> {
    let img = image::open(path).map_err(|e| {
        NarratorError::FrameExtractionError(format!("Failed to open frame {}: {e}", path.display()))
    })?;

    let (w, h) = (img.width(), img.height());
    let img = if w > max_width {
        let new_h = (h as f64 * max_width as f64 / w as f64).round() as u32;
        img.resize(max_width, new_h, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };

    let mut buf = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut buf);
    img.write_to(&mut cursor, image::ImageFormat::Jpeg)
        .map_err(|e| NarratorError::FrameExtractionError(format!("JPEG encode failed: {e}")))?;

    Ok(base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        &buf,
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
        // 0/1 returns safe default of 30.0 to prevent division-by-zero downstream
        assert!((parse_frame_rate("0/1") - 30.0).abs() < 0.01);
    }

    #[test]
    fn test_detect_ffmpeg() {
        // This test will pass if ffmpeg is installed
        if let Ok(path) = detect_ffmpeg() {
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

    #[test]
    fn test_parse_frame_rate_edge_cases() {
        // Empty string falls through to parse::<f64> which fails, returns safe default 30.0
        assert!((parse_frame_rate("") - 30.0).abs() < 0.01);

        // Single number (no slash) should parse directly
        assert!((parse_frame_rate("25") - 25.0).abs() < 0.01);
        assert!((parse_frame_rate("60") - 60.0).abs() < 0.01);

        // Negative values: num < 0 so fps < 0, returns safe default
        assert!((parse_frame_rate("-30/1") - 30.0).abs() < 0.01);

        // Very large values: fps >= 1000, returns safe default
        assert!((parse_frame_rate("100000/1") - 30.0).abs() < 0.01);

        // Malformed strings like "abc/def": parse fails, falls through to default
        assert!((parse_frame_rate("abc/def") - 30.0).abs() < 0.01);

        // Denominator 0: den is 0.0 which is not > 0.0, falls through to default
        assert!((parse_frame_rate("30/0") - 30.0).abs() < 0.01);

        // Valid edge: very small fps
        assert!((parse_frame_rate("1/10") - 0.1).abs() < 0.01);
    }

    #[test]
    fn test_deduplicate_frames_all_same() {
        // All frames point to the same nonexistent path, so hash_frame_file returns ""
        // for all of them. Since all hashes are equal, only the first frame survives.
        let frames = vec![
            Frame {
                index: 0,
                timestamp_seconds: 0.0,
                path: PathBuf::from("/nonexistent_same"),
                width: 100,
                height: 100,
            },
            Frame {
                index: 1,
                timestamp_seconds: 1.0,
                path: PathBuf::from("/nonexistent_same"),
                width: 100,
                height: 100,
            },
            Frame {
                index: 2,
                timestamp_seconds: 2.0,
                path: PathBuf::from("/nonexistent_same"),
                width: 100,
                height: 100,
            },
        ];
        let result = deduplicate_frames(frames);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].index, 0);
        assert!((result[0].timestamp_seconds - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_frame_to_base64_nonexistent() {
        let result = frame_to_base64(Path::new("/nonexistent/frame.jpg"));
        assert!(result.is_err());
    }
}
