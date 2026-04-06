//! Screen recording: macOS native screencapture, Windows ffmpeg gdigrab with overlay.

use crate::error::NarratorError;
use crate::video_engine;
use std::path::PathBuf;

/// Returns `~/Documents/Narrator/Recordings/`, creating it if needed.
pub fn get_recordings_dir() -> Result<PathBuf, NarratorError> {
    let dir = if let Some(user_dirs) = directories::UserDirs::new() {
        if let Some(doc_dir) = user_dirs.document_dir() {
            doc_dir.join("Narrator").join("Recordings")
        } else {
            default_recordings_dir()
        }
    } else {
        default_recordings_dir()
    };
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn default_recordings_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home)
        .join("Documents")
        .join("Narrator")
        .join("Recordings")
}

// ── macOS: native screencapture ──

/// Opens the macOS Cmd+Shift+5 screen recording UI. Blocks until the user stops recording.
#[cfg(target_os = "macos")]
pub async fn record_native(output_path: &str) -> Result<String, NarratorError> {
    if let Some(parent) = PathBuf::from(output_path).parent() {
        std::fs::create_dir_all(parent)?;
    }

    tracing::info!("Starting native macOS screen recording to {output_path}");

    let output = tokio::process::Command::new("screencapture")
        .args(["-J", "video", output_path])
        .output()
        .await
        .map_err(|e| NarratorError::FfmpegFailed(format!("screencapture failed: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.is_empty() {
            return Err(NarratorError::Cancelled);
        }
        return Err(NarratorError::FfmpegFailed(format!(
            "screencapture error: {stderr}"
        )));
    }

    if !PathBuf::from(output_path).exists() {
        return Err(NarratorError::Cancelled);
    }

    Ok(output_path.to_string())
}

#[cfg(not(target_os = "macos"))]
pub async fn record_native(_output_path: &str) -> Result<String, NarratorError> {
    Err(NarratorError::FfmpegFailed(
        "Native screen recording is only supported on macOS. Use start_screen_recording on Windows."
            .into(),
    ))
}

// ── Windows: ffmpeg gdigrab segment-based recording ──

/// Start an ffmpeg gdigrab recording segment. Returns the child process handle.
#[cfg(target_os = "windows")]
pub async fn start_segment(
    output_dir: &str,
    segment_index: u32,
) -> Result<(tokio::process::Child, String), NarratorError> {
    let ffmpeg = video_engine::detect_ffmpeg()?;
    let segment_path = PathBuf::from(output_dir)
        .join(format!("segment_{segment_index}.mp4"))
        .to_string_lossy()
        .to_string();

    tracing::info!("Starting recording segment {segment_index} → {segment_path}");

    let child = tokio::process::Command::new(ffmpeg.as_os_str())
        .args([
            "-y",
            "-f",
            "gdigrab",
            "-framerate",
            "30",
            "-i",
            "desktop",
            "-vcodec",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            &segment_path,
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| NarratorError::FfmpegFailed(format!("Failed to start recording: {e}")))?;

    Ok((child, segment_path))
}

/// Stub for non-Windows platforms.
#[cfg(not(target_os = "windows"))]
pub async fn start_segment(
    _output_dir: &str,
    _segment_index: u32,
) -> Result<(tokio::process::Child, String), NarratorError> {
    Err(NarratorError::FfmpegFailed(
        "Segment-based recording is only supported on Windows.".into(),
    ))
}

/// Gracefully stop an ffmpeg recording segment by sending 'q' to stdin.
pub async fn stop_segment(child: &mut tokio::process::Child) -> Result<(), NarratorError> {
    use tokio::io::AsyncWriteExt;

    if let Some(stdin) = child.stdin.as_mut() {
        let _ = stdin.write_all(b"q").await;
        let _ = stdin.flush().await;
    }

    // Wait up to 5 seconds for graceful exit, then kill
    match tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            tracing::warn!("ffmpeg wait error: {e}");
        }
        Err(_) => {
            tracing::warn!("ffmpeg did not exit in 5s, killing");
            let _ = child.kill().await;
        }
    }

    Ok(())
}

/// Concatenate multiple recording segments into a single output file.
/// If there's only one segment, just rename it.
pub async fn concatenate_segments(
    segments: &[String],
    output_path: &str,
) -> Result<String, NarratorError> {
    if segments.is_empty() {
        return Err(NarratorError::FfmpegFailed(
            "No recording segments to concatenate".into(),
        ));
    }

    if segments.len() == 1 {
        // Single segment — just move it
        std::fs::rename(&segments[0], output_path).map_err(|e| {
            NarratorError::FfmpegFailed(format!("Failed to move segment to output: {e}"))
        })?;
        return Ok(output_path.to_string());
    }

    // Multiple segments — use ffmpeg concat demuxer
    let ffmpeg = video_engine::detect_ffmpeg()?;
    let parent = PathBuf::from(output_path)
        .parent()
        .unwrap_or(&PathBuf::from("."))
        .to_path_buf();
    let filelist_path = parent.join("concat_list.txt");

    // Write the concat file list
    let filelist_content: String = segments
        .iter()
        .map(|s| format!("file '{s}'"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&filelist_path, &filelist_content)?;

    tracing::info!(
        "Concatenating {} segments into {output_path}",
        segments.len()
    );

    let output = tokio::process::Command::new(ffmpeg.as_os_str())
        .args([
            "-y",
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
            &filelist_path.to_string_lossy(),
            "-c",
            "copy",
            output_path,
        ])
        .output()
        .await
        .map_err(|e| NarratorError::FfmpegFailed(format!("Concat failed: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(NarratorError::FfmpegFailed(format!(
            "Concat error: {stderr}"
        )));
    }

    // Clean up segment files and filelist
    let _ = std::fs::remove_file(&filelist_path);
    for seg in segments {
        let _ = std::fs::remove_file(seg);
    }

    Ok(output_path.to_string())
}

// ── Windows: overlay capture exclusion ──

/// Mark a window as excluded from screen capture using SetWindowDisplayAffinity.
/// Available on Windows 10 version 2004+.
#[cfg(target_os = "windows")]
pub fn set_window_display_affinity(hwnd: isize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::SetWindowDisplayAffinity;

    // WDA_EXCLUDEFROMCAPTURE = 0x00000011 (Windows 10 2004+)
    // WDA_MONITOR = 0x00000001 (fallback, shows black in captures)
    const WDA_EXCLUDEFROMCAPTURE: u32 = 0x00000011;
    const WDA_MONITOR: u32 = 0x00000001;

    let result = unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) };
    if result == 0 {
        tracing::warn!("WDA_EXCLUDEFROMCAPTURE not supported, falling back to WDA_MONITOR");
        unsafe {
            SetWindowDisplayAffinity(hwnd, WDA_MONITOR);
        }
    }
}

#[cfg(not(target_os = "windows"))]
#[allow(dead_code)]
pub fn set_window_display_affinity(_hwnd: isize) {
    // No-op on non-Windows
}
