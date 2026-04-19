//! Video editing operations: validation + the public render API
//! (`apply_edits`, `merge_audio_video`, `burn_subtitles`,
//! `extract_single_frame`, `extract_edit_thumbnails`).
//!
//! After the Phase 3+4 rewrite, the heavy lifting (decode → composite →
//! encode for clips and overlay effects) lives in `crate::compositor`.
//! This module is now mostly the public surface + a few ffmpeg fast paths
//! that don't need the compositor (single-clip stream-copy trim,
//! subtitle-burn pass-through, etc.).

use crate::error::NarratorError;
use crate::models::ZoomPanEffect;
use crate::process_utils::CommandNoWindow;
use crate::video_engine;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

// ── Validation helpers (S1–S5) ──

/// Validate a video path blocks obvious traversal attacks but allows user-selected files.
/// Files selected via native dialog can be anywhere on disk — we only reject paths that
/// try to escape via `..` components or point at system-critical directories.
fn validate_path(p: &str) -> Result<PathBuf, NarratorError> {
    let path = PathBuf::from(p);

    // Block raw ".." components (path traversal)
    for component in path.components() {
        if let std::path::Component::ParentDir = component {
            return Err(NarratorError::ExportError(format!(
                "Path contains '..': {p}"
            )));
        }
    }

    // Block system-critical paths (Unix)
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(canonical) = std::fs::canonicalize(&path) {
            let s = canonical.to_string_lossy();
            if s.starts_with("/etc")
                || s.starts_with("/bin")
                || s.starts_with("/sbin")
                || s.starts_with("/usr/bin")
                || s.starts_with("/usr/sbin")
                || s.starts_with("/System")
            {
                return Err(NarratorError::ExportError(format!("Path not allowed: {p}")));
            }
        }
    }

    Ok(path)
}

/// Validate clip parameters (S2: DoS, F4: bounds).
fn validate_clip(clip: &EditClip, duration: f64, index: usize) -> Result<(), NarratorError> {
    let err = |msg: &str| NarratorError::ExportError(format!("Clip {index}: {msg}"));
    // Reject NaN / Infinity — they produce invalid ffmpeg args and cause hangs or crashes.
    if !clip.speed.is_finite() || !clip.start_seconds.is_finite() || !clip.end_seconds.is_finite() {
        return Err(err("speed/start/end must be finite"));
    }
    if clip.speed <= 0.0 || clip.speed > 100.0 {
        return Err(err(&format!("speed {} out of range (0, 100]", clip.speed)));
    }
    if clip.start_seconds < -0.1 {
        return Err(err("start_seconds is negative"));
    }
    if clip.end_seconds < clip.start_seconds {
        return Err(err("end_seconds < start_seconds"));
    }
    // Reject zero/near-zero duration clips except freeze (which sets its own duration).
    let source_dur = clip.end_seconds - clip.start_seconds;
    let is_freeze = clip.clip_type.as_deref() == Some("freeze");
    if !is_freeze && source_dur < 0.05 {
        return Err(err(&format!(
            "clip duration {:.3}s too short (min 0.05s)",
            source_dur
        )));
    }
    if clip.end_seconds > duration + 5.0 {
        return Err(err(&format!(
            "end_seconds {:.1} exceeds video duration {:.1}",
            clip.end_seconds, duration
        )));
    }
    if let Some(fps) = clip.fps_override {
        if !fps.is_finite() || fps <= 0.0 || fps > 240.0 {
            return Err(err(&format!("fps_override {} out of range (0, 240]", fps)));
        }
    }
    if let Some(fd) = clip.freeze_duration {
        if !fd.is_finite() || fd <= 0.0 || fd > 600.0 {
            return Err(err(&format!(
                "freeze_duration {} out of range (0, 600]",
                fd
            )));
        }
    }
    if let Some(fst) = clip.freeze_source_time {
        if !fst.is_finite() || fst < 0.0 || fst > duration + 1.0 {
            return Err(err(&format!(
                "freeze_source_time {} out of video range",
                fst
            )));
        }
    }
    Ok(())
}

/// Validate zoom regions (S4: NaN/Infinity).
fn validate_zoom(zp: &ZoomPanEffect) -> Result<(), NarratorError> {
    let err = |field: &str| NarratorError::ExportError(format!("Zoom region has invalid {field}"));
    for (label, r) in [("start", &zp.start_region), ("end", &zp.end_region)] {
        if !r.x.is_finite() || !r.y.is_finite() || !r.width.is_finite() || !r.height.is_finite() {
            return Err(err(&format!("{label} region values")));
        }
        if r.width <= 0.0 || r.height <= 0.0 {
            return Err(err(&format!("{label} region size")));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoEditPlan {
    pub clips: Vec<EditClip>,
    #[serde(default)]
    pub effects: Option<Vec<OverlayEffect>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayEffect {
    #[serde(rename = "type")]
    pub effect_type: String,
    pub start_time: f64,
    pub end_time: f64,
    #[serde(default)]
    pub transition_in: Option<f64>,
    #[serde(default)]
    pub transition_out: Option<f64>,
    #[serde(default)]
    pub reverse: Option<bool>,
    #[serde(default)]
    pub spotlight: Option<SpotlightData>,
    #[serde(default)]
    pub blur: Option<BlurData>,
    #[serde(default)]
    pub text: Option<TextData>,
    #[serde(default)]
    pub fade: Option<FadeData>,
    /// Present when effect_type == "zoom-pan". Carries the start/end regions
    /// and easing. Unlike the legacy per-clip zoom_pan on EditClip, this one
    /// is animated over its own [start_time, end_time] window.
    #[serde(default)]
    pub zoom_pan: Option<ZoomPanEffect>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpotlightData {
    pub x: f64,
    pub y: f64,
    pub radius: f64,
    pub dim_opacity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlurData {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub radius: f64,
    #[serde(default)]
    pub invert: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextData {
    pub content: String,
    pub x: f64,
    pub y: f64,
    pub font_size: f64,
    pub color: String,
    #[serde(default)]
    pub font_family: Option<String>,
    #[serde(default)]
    pub bold: Option<bool>,
    #[serde(default)]
    pub italic: Option<bool>,
    #[serde(default)]
    pub underline: Option<bool>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub align: Option<String>,
    #[serde(default)]
    pub opacity: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FadeData {
    pub color: String,
    pub opacity: f64,
}

/// One contiguous segment of source video on the output timeline.
/// Field names use snake_case (no `serde(rename_all)`) — the frontend
/// `VideoEditPlan.clips` shape relies on it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditClip {
    pub start_seconds: f64,
    pub end_seconds: f64,
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
    #[serde(default)]
    pub zoom_pan: Option<ZoomPanEffect>,
}

/// Run an ffmpeg command with real-time progress reporting.
/// Parses stderr for `time=` values and reports progress as 0.0-100.0.
async fn run_ffmpeg_with_progress(
    ffmpeg: &Path,
    args: &[&str],
    total_duration: f64,
    on_progress: &impl Fn(f64),
) -> Result<(), NarratorError> {
    // `-progress pipe:2` emits structured, newline-terminated progress events
    // (`out_time=HH:MM:SS.xxx`) to stderr so our line reader can parse them.
    // Default `frame=... time=...` stats use `\r` between updates, which
    // tokio's `lines()` doesn't split on — progress would never stream.
    // `-nostats` suppresses the default `\r`-terminated line. Both are global
    // options and must precede any output URL, so we prepend them.
    let mut full_args: Vec<&str> = vec!["-progress", "pipe:2", "-nostats"];
    full_args.extend_from_slice(args);
    let mut cmd = Command::new(ffmpeg.as_os_str());
    cmd.no_window()
        .args(&full_args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| NarratorError::FfmpegFailed(format!("Failed to start ffmpeg: {e}")))?;

    // Ring buffer of recent stderr lines so failures can include meaningful
    // context instead of just "exited with status 1". ffmpeg banners are
    // chatty; 40 lines is enough to catch the actual error tail.
    const STDERR_TAIL: usize = 40;
    let mut recent_stderr: std::collections::VecDeque<String> =
        std::collections::VecDeque::with_capacity(STDERR_TAIL + 1);

    if let Some(stderr) = child.stderr.take() {
        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(time_str) = extract_time_from_ffmpeg_line(&line) {
                let seconds = parse_ffmpeg_time(&time_str);
                if total_duration > 0.0 && seconds > 0.0 {
                    let pct = (seconds / total_duration * 100.0).min(100.0);
                    on_progress(pct);
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
        .map_err(|e| NarratorError::FfmpegFailed(format!("ffmpeg process error: {e}")))?;

    if !status.success() {
        // Surface the most relevant stderr lines (those that look like errors)
        // falling back to the tail if nothing obvious stands out.
        let tail: Vec<&String> = recent_stderr.iter().collect();
        let meaningful: String = tail
            .iter()
            .filter(|l| {
                let ll = l.to_lowercase();
                ll.contains("error")
                    || ll.contains("invalid")
                    || ll.contains("no such")
                    || ll.contains("failed")
                    || ll.contains("unknown")
                    || ll.contains("unrecognized")
                    || ll.contains("does not")
            })
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        let detail = if meaningful.is_empty() {
            tail.iter()
                .rev()
                .take(5)
                .rev()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        } else {
            meaningful
        };
        return Err(NarratorError::FfmpegFailed(format!(
            "ffmpeg exited with status {status}: {detail}"
        )));
    }

    on_progress(100.0);
    Ok(())
}

/// Extract the time= value from an ffmpeg stderr line.
///
/// Handles both formats:
/// - Structured `-progress pipe:2` output: `out_time=00:00:01.666000\n`
///   (and `out_time_us=...`, `out_time_ms=...` — we prefer `out_time=`).
/// - Legacy `-stats` output: `frame=100 ... time=00:00:01.66 bitrate=...\r`.
fn extract_time_from_ffmpeg_line(line: &str) -> Option<String> {
    // Prefer the structured `-progress` format; it's \n-terminated so lines()
    // can actually see it.
    if let Some(i) = line.find("out_time=") {
        let rest = &line[i + 9..];
        let end = rest.find([' ', '\n', '\r']).unwrap_or(rest.len());
        let time_str = rest[..end].trim();
        if time_str.is_empty() || time_str == "N/A" {
            return None;
        }
        return Some(time_str.to_string());
    }
    // Fallback: legacy stats line. This only fires in contexts where we didn't
    // pass -nostats (we always do now, but keep the fallback for safety).
    let time_idx = line.find("time=")?;
    let rest = &line[time_idx + 5..];
    let end = rest.find(' ').unwrap_or(rest.len());
    let time_str = &rest[..end];
    if time_str == "N/A" {
        return None;
    }
    Some(time_str.to_string())
}

/// Parse ffmpeg time format "HH:MM:SS.ms" to seconds.
fn parse_ffmpeg_time(time_str: &str) -> f64 {
    let parts: Vec<&str> = time_str.split(':').collect();
    match parts.len() {
        3 => {
            let hours: f64 = parts[0].parse().unwrap_or(0.0);
            let minutes: f64 = parts[1].parse().unwrap_or(0.0);
            let seconds: f64 = parts[2].parse().unwrap_or(0.0);
            hours * 3600.0 + minutes * 60.0 + seconds
        }
        2 => {
            let minutes: f64 = parts[0].parse().unwrap_or(0.0);
            let seconds: f64 = parts[1].parse().unwrap_or(0.0);
            minutes * 60.0 + seconds
        }
        1 => parts[0].parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

/// Extract a single frame from a video at a given timestamp.
pub async fn extract_single_frame(
    video_path: &str,
    timestamp: f64,
    output_path: &str,
) -> Result<String, NarratorError> {
    let ffmpeg = video_engine::detect_ffmpeg()?;

    let output = Command::new(ffmpeg.as_os_str())
        .no_window()
        .args([
            "-y",
            "-ss",
            &format!("{:.3}", timestamp),
            "-i",
            video_path,
            "-frames:v",
            "1",
            "-q:v",
            "2",
            output_path,
        ])
        .output()
        .await
        .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(NarratorError::FfmpegFailed(format!(
            "Frame extraction failed: {}",
            &stderr[stderr.len().saturating_sub(500)..]
        )));
    }

    Ok(output_path.to_string())
}
/// Render an edit plan (clips + overlay effects) to a single MP4 at
/// `output_path`. Validation + a couple of single-clip ffmpeg fast paths
/// live here; everything else hands off to the in-process compositor.
pub async fn apply_edits(
    input_path: &str,
    output_path: &str,
    plan: &VideoEditPlan,
    on_progress: impl Fn(f64) + Send + Sync,
) -> Result<String, NarratorError> {
    validate_path(input_path)?;
    validate_path(output_path)?;

    let total = plan.clips.len();
    if total == 0 {
        return Err(NarratorError::ExportError("No clips to process".into()));
    }

    let meta = video_engine::probe_video(Path::new(input_path)).await?;

    let out_dir = Path::new(output_path).parent().unwrap_or(Path::new("/tmp"));
    tokio::fs::create_dir_all(out_dir).await.map_err(|e| {
        NarratorError::ExportError(format!(
            "Failed to create output directory {}: {e}",
            out_dir.display()
        ))
    })?;

    for (i, clip) in plan.clips.iter().enumerate() {
        validate_clip(clip, meta.duration_seconds, i)?;
        if let Some(ref zp) = clip.zoom_pan {
            validate_zoom(zp)?;
        }
    }

    // Two ffmpeg fast paths for the trivial single-clip cases — these
    // skip the compositor entirely (no decode/encode round-trip) and just
    // copy or stream-trim the source. They cover the common "preview /
    // review" case where the user has not edited anything yet.
    let first = &plan.clips[0];
    let has_clip_fx = first.clip_type.as_deref() == Some("freeze") || first.zoom_pan.is_some();
    let has_overlay_fx = plan
        .effects
        .as_ref()
        .map(|v| {
            v.iter().any(|e| {
                matches!(
                    e.effect_type.as_str(),
                    "spotlight" | "blur" | "text" | "fade" | "zoom-pan"
                )
            })
        })
        .unwrap_or(false);
    let trivial_single = total == 1
        && first.speed == 1.0
        && first.fps_override.is_none()
        && !has_clip_fx
        && !has_overlay_fx;

    if trivial_single {
        let covers_full =
            first.start_seconds < 0.5 && (first.end_seconds - meta.duration_seconds).abs() < 0.5;
        if covers_full {
            if input_path != output_path {
                tokio::fs::copy(input_path, output_path).await?;
            }
            on_progress(100.0);
            return Ok(output_path.to_string());
        }

        // Single trimmed clip with no effects → ffmpeg stream-copy trim.
        let ffmpeg = video_engine::detect_ffmpeg()?;
        let duration = first.end_seconds - first.start_seconds;
        let output = Command::new(ffmpeg.as_os_str())
            .no_window()
            .args([
                "-y",
                "-ss",
                &format!("{:.3}", first.start_seconds),
                "-i",
                input_path,
                "-t",
                &format!("{:.3}", duration),
                "-c",
                "copy",
                "-avoid_negative_ts",
                "make_zero",
                output_path,
            ])
            .output()
            .await
            .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(NarratorError::FfmpegFailed(stderr.to_string()));
        }
        on_progress(100.0);
        return Ok(output_path.to_string());
    }

    // Anything else → the in-process compositor handles the whole render.
    crate::compositor::run_pipeline(
        Path::new(input_path),
        Path::new(output_path),
        plan,
        &on_progress,
    )
    .await?;
    on_progress(100.0);
    Ok(output_path.to_string())
}

pub async fn merge_audio_video(
    video_path: &str,
    audio_path: &str,
    output_path: &str,
    replace_audio: bool,
    on_progress: impl Fn(f64),
) -> Result<String, NarratorError> {
    let ffmpeg = video_engine::detect_ffmpeg()?;

    if replace_audio {
        // Replace original audio entirely with narration.
        // Uses -c:v copy so it's fast — no re-encoding needed.
        let output = Command::new(ffmpeg.as_os_str())
            .no_window()
            .args([
                "-y",
                "-i",
                video_path,
                "-i",
                audio_path,
                "-c:v",
                "copy",
                "-c:a",
                "aac",
                "-map",
                "0:v:0",
                "-map",
                "1:a:0",
                output_path,
            ])
            .output()
            .await
            .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(NarratorError::FfmpegFailed(format!(
                "Audio merge failed: {}",
                &stderr[stderr.len().saturating_sub(500)..]
            )));
        }

        on_progress(100.0);
        return Ok(output_path.to_string());
    }

    // Mix original + narration audio. Phase 5: this happens in Rust via the
    // compositor::audio mixer (sample-level math + optional ducking) instead
    // of ffmpeg amix. ffmpeg only does the final mux.
    on_progress(5.0);

    let tmp_dir = std::env::temp_dir();
    let mixed_wav = tmp_dir.join(format!("_narrator_mix_{}.wav", uuid::Uuid::new_v4()));
    let _cleanup = TempCleanup(vec![mixed_wav.clone()]);

    // Default ducking: -8dB whenever narration has signal. Keeps narration
    // intelligible without making the original disappear.
    let mix_result = crate::compositor::audio::mix_narration(
        Path::new(video_path),
        Path::new(audio_path),
        &mixed_wav,
        1.0,
        1.0,
        -8.0,
    )
    .await;

    if let Err(e) = mix_result {
        // Fallback: video might not have an audio stream — use narration only
        // via straight mux.
        tracing::warn!("Rust mix failed ({e}), falling back to narration-only mux");
        let fallback = Command::new(ffmpeg.as_os_str())
            .no_window()
            .args([
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-i",
                video_path,
                "-i",
                audio_path,
                "-c:v",
                "copy",
                "-c:a",
                "aac",
                "-map",
                "0:v:0",
                "-map",
                "1:a:0",
                output_path,
            ])
            .output()
            .await
            .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;
        if !fallback.status.success() {
            let stderr = String::from_utf8_lossy(&fallback.stderr);
            return Err(NarratorError::FfmpegFailed(format!(
                "Audio merge fallback failed: {}",
                &stderr[stderr.len().saturating_sub(500)..]
            )));
        }
        on_progress(100.0);
        return Ok(output_path.to_string());
    }

    on_progress(70.0);

    // Mux: video stream copied through, audio = the mixed WAV → AAC.
    let mux = Command::new(ffmpeg.as_os_str())
        .no_window()
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            video_path,
            "-i",
        ])
        .arg(&mixed_wav)
        .args([
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-c:v",
            "copy",
            "-c:a",
            "aac",
            "-b:a",
            "256k",
            "-movflags",
            "+faststart",
            output_path,
        ])
        .output()
        .await
        .map_err(|e| NarratorError::FfmpegFailed(format!("mux: {e}")))?;
    if !mux.status.success() {
        let stderr = String::from_utf8_lossy(&mux.stderr);
        return Err(NarratorError::FfmpegFailed(format!(
            "Audio mux failed: {}",
            &stderr[stderr.len().saturating_sub(500)..]
        )));
    }

    on_progress(100.0);
    Ok(output_path.to_string())
}

/// RAII helper for cleaning up temp files inside merge_audio_video.
struct TempCleanup(Vec<PathBuf>);
impl Drop for TempCleanup {
    fn drop(&mut self) {
        for p in &self.0 {
            let _ = std::fs::remove_file(p);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleStyle {
    pub font_size: u32,
    pub color: String,
    pub outline_color: String,
    pub outline: u32,
    pub position: String,
}

impl Default for SubtitleStyle {
    fn default() -> Self {
        Self {
            font_size: 22,
            color: "#ffffff".to_string(),
            outline_color: "#000000".to_string(),
            outline: 2,
            position: "bottom".to_string(),
        }
    }
}

/// Convert a hex RGB color string (e.g. "#ffffff") to ffmpeg ASS BGR format (e.g. "&H00FFFFFF").
/// ASS colour format is &HAABBGGRR where AA=alpha (00=opaque).
fn hex_rgb_to_ass_bgr(hex: &str) -> String {
    let hex = hex.trim_start_matches('#');
    if hex.len() == 8 {
        // Has alpha channel (RRGGBBAA) — convert to ASS &HAABBGGRR
        let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(255);
        let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(255);
        let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(255);
        let a = u8::from_str_radix(&hex[6..8], 16).unwrap_or(0);
        // ASS alpha is inverted: 00 = opaque, FF = transparent
        let ass_alpha = 255 - a;
        format!("&H{:02X}{:02X}{:02X}{:02X}", ass_alpha, b, g, r)
    } else if hex.len() >= 6 {
        let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(255);
        let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(255);
        let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(255);
        format!("&H00{:02X}{:02X}{:02X}", b, g, r)
    } else {
        "&H00FFFFFF".to_string()
    }
}

pub async fn burn_subtitles(
    video_path: &str,
    srt_path: &str,
    output_path: &str,
    style: &SubtitleStyle,
    on_progress: impl Fn(f64),
) -> Result<String, NarratorError> {
    let ffmpeg = video_engine::detect_ffmpeg()?;

    // Probe video duration for progress reporting
    let meta = video_engine::probe_video(Path::new(video_path)).await?;
    let total_duration = meta.duration_seconds;

    // Copy SRT to temp dir with a unique name to avoid path escaping issues
    // with ffmpeg's subtitles filter (chokes on colons, spaces, special chars)
    let temp_srt =
        std::env::temp_dir().join(format!("_narrator_burn_subs_{}.srt", uuid::Uuid::new_v4()));
    tokio::fs::copy(srt_path, &temp_srt).await?;

    // Convert hex colors to ASS BGR format
    let primary_colour = hex_rgb_to_ass_bgr(&style.color);
    let outline_colour = hex_rgb_to_ass_bgr(&style.outline_color);

    // Position: bottom uses MarginV=30, top uses MarginV=10 + Alignment=6 (top-center)
    let position_style = if style.position == "top" {
        "MarginV=10,Alignment=6".to_string()
    } else {
        "MarginV=30".to_string()
    };

    // Try subtitles filter first (requires libass), fall back to SRT input method
    let srt_path_str = temp_srt
        .to_string_lossy()
        .replace('\\', "/")
        .replace(':', "\\:");
    // Sanitize numeric parameters to prevent unexpected ffmpeg filter behavior
    let font_size = style.font_size.clamp(8, 72);
    let outline = style.outline.clamp(0, 10);

    let subtitle_filter = format!(
        "subtitles='{}':force_style='FontSize={},PrimaryColour={},OutlineColour={},Outline={},BackColour=&H80000000,Shadow=1,{}'",
        srt_path_str, font_size, primary_colour, outline_colour, outline, position_style
    );

    let result = run_ffmpeg_with_progress(
        &ffmpeg,
        &[
            "-y",
            "-i",
            video_path,
            "-vf",
            &subtitle_filter,
            "-c:a",
            "copy",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "0",
            output_path,
        ],
        total_duration,
        &on_progress,
    )
    .await;

    if result.is_err() {
        // Fallback: use SRT as an input stream and overlay with mov_text → drawtext
        tracing::warn!("subtitles filter failed, trying SRT input overlay fallback");
        let fallback = Command::new(ffmpeg.as_os_str())
            .no_window()
            .args([
                "-y",
                "-i",
                video_path,
                "-i",
                &temp_srt.to_string_lossy(),
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-crf",
                "0",
                "-c:a",
                "copy",
                "-c:s",
                "mov_text",
                "-metadata:s:s:0",
                "language=eng",
                output_path,
            ])
            .output()
            .await
            .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;

        if !fallback.status.success() {
            let stderr = String::from_utf8_lossy(&fallback.stderr);
            let _ = tokio::fs::remove_file(&temp_srt).await;
            return Err(NarratorError::FfmpegFailed(format!(
                "Subtitle burn failed: {}",
                &stderr[..stderr.len().min(400)]
            )));
        }
    }

    let _ = tokio::fs::remove_file(&temp_srt).await;
    on_progress(100.0);

    Ok(output_path.to_string())
}

pub async fn extract_edit_thumbnails(
    video_path: &str,
    output_dir: &str,
    count: usize,
) -> Result<Vec<String>, NarratorError> {
    // Cache hit: if the output dir already has ≥ count JPGs AND the source
    // video hasn't been modified since they were produced, return them
    // without re-running ffmpeg. Checked BEFORE probing so repeat calls
    // (e.g. navigating back to Edit Video) are near-instant.
    {
        let dir = output_dir.to_string();
        let video = video_path.to_string();
        let cached = tokio::task::spawn_blocking(move || -> Option<Vec<String>> {
            let video_mtime = std::fs::metadata(&video).ok()?.modified().ok()?;
            let entries: Vec<std::fs::DirEntry> = std::fs::read_dir(&dir)
                .ok()?
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|x| x == "jpg"))
                .collect();
            if entries.len() < count {
                return None;
            }
            for entry in &entries {
                let t_meta = entry.metadata().ok()?;
                let t_mtime = t_meta.modified().ok()?;
                if t_mtime < video_mtime {
                    return None;
                }
            }
            let mut paths: Vec<String> = entries
                .into_iter()
                .map(|e| e.path().to_string_lossy().to_string())
                .collect();
            paths.sort();
            Some(paths)
        })
        .await
        .ok()
        .flatten();
        if let Some(paths) = cached {
            tracing::info!(
                "extract_edit_thumbnails: cache hit ({} thumbs in {})",
                paths.len(),
                output_dir
            );
            return Ok(paths);
        }
    }

    let ffmpeg = video_engine::detect_ffmpeg()?;
    let meta = video_engine::probe_video(Path::new(video_path)).await?;
    let interval = meta.duration_seconds / count as f64;

    tokio::fs::create_dir_all(output_dir).await?;

    let output = Command::new(ffmpeg.as_os_str())
        .no_window()
        .args([
            "-y",
            "-i",
            video_path,
            "-vf",
            &format!(
                "fps=1/{:.3},scale='min(120,iw)':'min(68,ih)':force_original_aspect_ratio=decrease",
                interval
            ),
            "-q:v",
            "5",
            &format!("{}/thumb_%04d.jpg", output_dir),
        ])
        .output()
        .await
        .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(NarratorError::FfmpegFailed(stderr.to_string()));
    }

    let dir = output_dir.to_string();
    let paths = tokio::task::spawn_blocking(move || {
        let mut paths: Vec<String> = std::fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "jpg"))
            .map(|e| e.path().to_string_lossy().to_string())
            .collect();
        paths.sort();
        Ok::<_, std::io::Error>(paths)
    })
    .await
    .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))??;
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::EasingPreset;

    #[test]
    fn test_extract_time_from_ffmpeg_line() {
        assert_eq!(
            extract_time_from_ffmpeg_line(
                "frame=  120 fps=30 q=28.0 size=    1024kB time=00:01:30.50 bitrate= 2094.1kbits/s"
            ),
            Some("00:01:30.50".to_string())
        );
        assert_eq!(extract_time_from_ffmpeg_line("time=N/A"), None);
        assert_eq!(extract_time_from_ffmpeg_line("no time here"), None);
    }

    #[test]
    fn test_parse_ffmpeg_time() {
        assert!((parse_ffmpeg_time("00:01:30.50") - 90.5).abs() < 0.01);
        assert!((parse_ffmpeg_time("01:00:00.00") - 3600.0).abs() < 0.01);
        assert!((parse_ffmpeg_time("00:00:05.25") - 5.25).abs() < 0.01);
        assert!((parse_ffmpeg_time("") - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_parse_ffmpeg_time_two_parts() {
        assert!((parse_ffmpeg_time("01:30.00") - 90.0).abs() < 0.01);
    }

    #[test]
    fn test_extract_time_at_end_of_line() {
        // time= at end of line with no trailing space
        assert_eq!(
            extract_time_from_ffmpeg_line("size=1024kB time=00:00:10.00"),
            Some("00:00:10.00".to_string())
        );
    }

    #[test]
    fn test_atempo_chaining_for_high_speeds() {
        // Simulate the atempo chain logic for speed=10x
        let speed = 10.0_f64;
        let mut chain = Vec::new();
        let mut remaining = speed;
        while remaining > 2.0 {
            chain.push("atempo=2.0".to_string());
            remaining /= 2.0;
        }
        while remaining < 0.5 {
            chain.push("atempo=0.5".to_string());
            remaining /= 0.5;
        }
        chain.push(format!("atempo={:.4}", remaining));

        // 10 / 2 / 2 / 2 = 1.25 → needs 3x atempo=2.0 + 1x atempo=1.25
        assert_eq!(chain.len(), 4);
        assert_eq!(chain[0], "atempo=2.0");
        assert_eq!(chain[1], "atempo=2.0");
        assert_eq!(chain[2], "atempo=2.0");
        assert!(chain[3].starts_with("atempo=1.25"));
        // Product should equal original speed: 2 * 2 * 2 * 1.25 = 10
        let product: f64 = chain
            .iter()
            .map(|s| s.strip_prefix("atempo=").unwrap().parse::<f64>().unwrap())
            .product();
        assert!((product - speed).abs() < 0.01);
    }

    #[test]
    fn test_expected_output_duration() {
        // 20 seconds at 10x speed = 2 seconds output
        let clip_duration: f64 = 20.0;
        let speed: f64 = 10.0;
        let expected = clip_duration / speed;
        assert!((expected - 2.0).abs() < 0.001);

        // 60 seconds at 1x speed = 60 seconds output
        let expected_1x: f64 = 60.0 / 1.0;
        assert!((expected_1x - 60.0).abs() < 0.001);

        // 30 seconds at 3x speed = 10 seconds output
        let expected_3x: f64 = 30.0 / 3.0;
        assert!((expected_3x - 10.0).abs() < 0.001);
    }

    #[test]
    fn test_validate_clip_bounds() {
        let clip = EditClip {
            start_seconds: 10.0,
            end_seconds: 30.0,
            speed: 5.0,
            skip_frames: false,
            fps_override: None,
            clip_type: None,
            freeze_source_time: None,
            freeze_duration: None,
            zoom_pan: None,
        };
        assert!(validate_clip(&clip, 60.0, 0).is_ok());

        // Speed 0 should fail
        let bad_speed = EditClip {
            speed: 0.0,
            ..clip.clone()
        };
        assert!(validate_clip(&bad_speed, 60.0, 0).is_err());

        // end < start should fail
        let bad_range = EditClip {
            start_seconds: 30.0,
            end_seconds: 10.0,
            ..clip.clone()
        };
        assert!(validate_clip(&bad_range, 60.0, 0).is_err());
    }

    #[test]
    fn test_validate_clip_rejects_nan_infinity() {
        let base = EditClip {
            start_seconds: 0.0,
            end_seconds: 10.0,
            speed: 1.0,
            skip_frames: false,
            fps_override: None,
            clip_type: None,
            freeze_source_time: None,
            freeze_duration: None,
            zoom_pan: None,
        };
        assert!(validate_clip(
            &EditClip {
                speed: f64::NAN,
                ..base.clone()
            },
            60.0,
            0
        )
        .is_err());
        assert!(validate_clip(
            &EditClip {
                speed: f64::INFINITY,
                ..base.clone()
            },
            60.0,
            0
        )
        .is_err());
        assert!(validate_clip(
            &EditClip {
                start_seconds: f64::NAN,
                ..base.clone()
            },
            60.0,
            0
        )
        .is_err());
        assert!(validate_clip(
            &EditClip {
                end_seconds: f64::NAN,
                ..base
            },
            60.0,
            0
        )
        .is_err());
    }

    #[test]
    fn test_validate_clip_rejects_zero_duration() {
        let clip = EditClip {
            start_seconds: 5.0,
            end_seconds: 5.0,
            speed: 1.0,
            skip_frames: false,
            fps_override: None,
            clip_type: None,
            freeze_source_time: None,
            freeze_duration: None,
            zoom_pan: None,
        };
        assert!(validate_clip(&clip, 60.0, 0).is_err());
    }

    #[test]
    fn test_validate_clip_allows_freeze_zero_source() {
        // Freeze clip source span can be zero — duration comes from freeze_duration
        let clip = EditClip {
            start_seconds: 5.0,
            end_seconds: 5.0,
            speed: 1.0,
            skip_frames: false,
            fps_override: None,
            clip_type: Some("freeze".into()),
            freeze_source_time: Some(5.0),
            freeze_duration: Some(3.0),
            zoom_pan: None,
        };
        assert!(validate_clip(&clip, 60.0, 0).is_ok());
    }

    #[test]
    fn test_validate_clip_rejects_bad_fps_override() {
        let base = EditClip {
            start_seconds: 0.0,
            end_seconds: 10.0,
            speed: 1.0,
            skip_frames: false,
            fps_override: Some(1000.0),
            clip_type: None,
            freeze_source_time: None,
            freeze_duration: None,
            zoom_pan: None,
        };
        assert!(validate_clip(&base, 60.0, 0).is_err());

        let nan = EditClip {
            fps_override: Some(f64::NAN),
            ..base.clone()
        };
        assert!(validate_clip(&nan, 60.0, 0).is_err());
    }

    #[test]
    fn test_validate_clip_rejects_bad_freeze_source_time() {
        let clip = EditClip {
            start_seconds: 0.0,
            end_seconds: 2.0,
            speed: 1.0,
            skip_frames: false,
            fps_override: None,
            clip_type: Some("freeze".into()),
            freeze_source_time: Some(999.0),
            freeze_duration: Some(3.0),
            zoom_pan: None,
        };
        assert!(validate_clip(&clip, 60.0, 0).is_err());
    }

    #[test]
    fn test_validate_zoom_rejects_nan() {
        let bad = ZoomPanEffect {
            start_region: crate::models::ZoomRegion {
                x: f64::NAN,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            end_region: crate::models::ZoomRegion {
                x: 0.0,
                y: 0.0,
                width: 0.5,
                height: 0.5,
            },
            easing: EasingPreset::Linear,
        };
        assert!(validate_zoom(&bad).is_err());
    }

    #[test]
    fn test_validate_zoom_rejects_zero_size() {
        let bad = ZoomPanEffect {
            start_region: crate::models::ZoomRegion {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 1.0,
            },
            end_region: crate::models::ZoomRegion {
                x: 0.0,
                y: 0.0,
                width: 0.5,
                height: 0.5,
            },
            easing: EasingPreset::Linear,
        };
        assert!(validate_zoom(&bad).is_err());
    }

    // ── Integration tests (run real ffmpeg) ────────────────────────
    //
    // These tests create a short test video with `lavfi testsrc` and an audio
    // sine tone, then run the real editing pipeline. They take a few seconds
    // each and require ffmpeg on PATH. If ffmpeg isn't available the test is
    // reported as skipped (early return).

    fn ffmpeg_ok() -> bool {
        video_engine::detect_ffmpeg().is_ok()
    }

    /// Some ffmpeg builds (notably Homebrew's default) ship without libfreetype,
    /// so the `drawtext` filter isn't registered. Tests that exercise text
    /// overlays skip themselves when this is the case.
    fn drawtext_available() -> bool {
        let Ok(ffmpeg) = video_engine::detect_ffmpeg() else {
            return false;
        };
        let Ok(output) = std::process::Command::new(ffmpeg).arg("-filters").output() else {
            return false;
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout.contains("drawtext")
    }

    async fn make_test_video(path: &Path, duration_s: f64) -> Result<(), String> {
        let ffmpeg = video_engine::detect_ffmpeg().map_err(|e| e.to_string())?;
        let output = Command::new(&ffmpeg)
            .no_window()
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                &format!("testsrc=duration={duration_s}:size=320x240:rate=30"),
                "-f",
                "lavfi",
                "-i",
                &format!("sine=frequency=1000:duration={duration_s}"),
                "-c:v",
                "libx264",
                // Force keyframe every 15 frames (0.5s) so trims are accurate when
                // the fast-path stream-copy seeks to the nearest keyframe.
                "-g",
                "15",
                "-keyint_min",
                "15",
                "-c:a",
                "aac",
                "-pix_fmt",
                "yuv420p",
                "-shortest",
            ])
            .arg(path)
            .output()
            .await
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(format!("make_test_video failed: {err}"));
        }
        Ok(())
    }

    fn simple_clip(start: f64, end: f64, speed: f64) -> EditClip {
        EditClip {
            start_seconds: start,
            end_seconds: end,
            speed,
            skip_frames: false,
            fps_override: None,
            clip_type: None,
            freeze_source_time: None,
            freeze_duration: None,
            zoom_pan: None,
        }
    }

    fn base_effect(kind: &str, start: f64, end: f64) -> OverlayEffect {
        OverlayEffect {
            effect_type: kind.to_string(),
            start_time: start,
            end_time: end,
            transition_in: None,
            transition_out: None,
            reverse: None,
            spotlight: None,
            blur: None,
            text: None,
            fade: None,
            zoom_pan: None,
        }
    }

    async fn apply_and_probe(
        plan: VideoEditPlan,
        input_duration: f64,
    ) -> Result<(f64, u32, u32), String> {
        let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
        let input = dir.path().join("input.mp4");
        let output = dir.path().join("output.mp4");
        make_test_video(&input, input_duration).await?;
        apply_edits(
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            &plan,
            |_| {},
        )
        .await
        .map_err(|e| e.to_string())?;
        let meta = video_engine::probe_video(&output)
            .await
            .map_err(|e| e.to_string())?;
        Ok((meta.duration_seconds, meta.width, meta.height))
    }

    #[tokio::test]
    async fn integration_trim_preserves_duration() {
        if !ffmpeg_ok() {
            return;
        }
        let plan = VideoEditPlan {
            clips: vec![simple_clip(2.0, 6.0, 1.0)],
            effects: None,
        };
        let (dur, w, h) = apply_and_probe(plan, 10.0).await.unwrap();
        assert!((dur - 4.0).abs() < 0.3, "expected ~4s, got {dur}s");
        assert_eq!((w, h), (320, 240));
    }

    #[tokio::test]
    async fn integration_speed_2x_halves_duration() {
        if !ffmpeg_ok() {
            return;
        }
        // 6s of source at 2x should produce a 3s output
        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 6.0, 2.0)],
            effects: None,
        };
        let (dur, _, _) = apply_and_probe(plan, 10.0).await.unwrap();
        assert!(
            (dur - 3.0).abs() < 0.3,
            "expected ~3s at 2x speed, got {dur}s"
        );
    }

    #[tokio::test]
    async fn integration_concat_multiple_clips() {
        if !ffmpeg_ok() {
            return;
        }
        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 2.0, 1.0), simple_clip(4.0, 6.0, 1.0)],
            effects: None,
        };
        let (dur, _, _) = apply_and_probe(plan, 10.0).await.unwrap();
        assert!((dur - 4.0).abs() < 0.5, "expected ~4s concat, got {dur}s");
    }

    #[tokio::test]
    async fn integration_zoom_pan() {
        if !ffmpeg_ok() {
            return;
        }
        let mut clip = simple_clip(0.0, 4.0, 1.0);
        clip.zoom_pan = Some(ZoomPanEffect {
            start_region: crate::models::ZoomRegion {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            end_region: crate::models::ZoomRegion {
                x: 0.25,
                y: 0.25,
                width: 0.5,
                height: 0.5,
            },
            easing: EasingPreset::EaseInOut,
        });
        let plan = VideoEditPlan {
            clips: vec![clip],
            effects: None,
        };
        let (dur, _, _) = apply_and_probe(plan, 10.0).await.unwrap();
        assert!(dur > 3.5 && dur < 4.5, "expected ~4s with zoom, got {dur}s");
    }

    #[tokio::test]
    async fn integration_blur_effect() {
        if !ffmpeg_ok() {
            return;
        }
        let mut fx = base_effect("blur", 0.5, 2.5);
        fx.blur = Some(BlurData {
            x: 0.25,
            y: 0.25,
            width: 0.5,
            height: 0.5,
            radius: 15.0,
            invert: None,
        });
        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 3.0, 1.0)],
            effects: Some(vec![fx]),
        };
        let (dur, _, _) = apply_and_probe(plan, 10.0).await.unwrap();
        assert!(dur > 2.7 && dur < 3.3, "expected ~3s with blur, got {dur}s");
    }

    #[tokio::test]
    async fn integration_spotlight_effect() {
        if !ffmpeg_ok() {
            return;
        }
        let mut fx = base_effect("spotlight", 0.0, 2.0);
        fx.spotlight = Some(SpotlightData {
            x: 0.5,
            y: 0.5,
            radius: 0.25,
            dim_opacity: 0.6,
        });
        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 3.0, 1.0)],
            effects: Some(vec![fx]),
        };
        let (dur, _, _) = apply_and_probe(plan, 10.0).await.unwrap();
        assert!(
            dur > 2.7 && dur < 3.3,
            "expected ~3s with spotlight, got {dur}s"
        );
    }

    #[tokio::test]
    async fn integration_text_effect() {
        if !ffmpeg_ok() || !drawtext_available() {
            return;
        }
        let mut fx = base_effect("text", 0.5, 2.5);
        fx.text = Some(TextData {
            content: "Test Overlay".into(),
            x: 0.5,
            y: 0.1,
            font_size: 5.0,
            color: "#ffffff".into(),
            font_family: None,
            bold: Some(true),
            italic: None,
            underline: None,
            background: Some("#000000".into()),
            align: None,
            opacity: Some(0.9),
        });
        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 3.0, 1.0)],
            effects: Some(vec![fx]),
        };
        let (dur, _, _) = apply_and_probe(plan, 10.0).await.unwrap();
        assert!(dur > 2.7 && dur < 3.3, "expected ~3s with text, got {dur}s");
    }

    #[tokio::test]
    async fn integration_fade_effect() {
        if !ffmpeg_ok() {
            return;
        }
        let mut fx = base_effect("fade", 1.0, 2.0);
        fx.fade = Some(FadeData {
            color: "#000000".into(),
            opacity: 0.7,
        });
        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 3.0, 1.0)],
            effects: Some(vec![fx]),
        };
        let (dur, _, _) = apply_and_probe(plan, 10.0).await.unwrap();
        assert!(dur > 2.7 && dur < 3.3, "expected ~3s with fade, got {dur}s");
    }

    #[tokio::test]
    async fn integration_multiple_effects_chained() {
        if !ffmpeg_ok() || !drawtext_available() {
            return;
        }
        let mut blur = base_effect("blur", 0.0, 3.0);
        blur.blur = Some(BlurData {
            x: 0.0,
            y: 0.0,
            width: 0.3,
            height: 0.3,
            radius: 10.0,
            invert: None,
        });
        let mut text = base_effect("text", 0.0, 3.0);
        text.text = Some(TextData {
            content: "Chained".into(),
            x: 0.5,
            y: 0.5,
            font_size: 8.0,
            color: "#ffffff".into(),
            font_family: None,
            bold: None,
            italic: None,
            underline: None,
            background: None,
            align: None,
            opacity: None,
        });
        let mut fade = base_effect("fade", 0.0, 3.0);
        fade.fade = Some(FadeData {
            color: "#ffffff".into(),
            opacity: 0.2,
        });
        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 3.0, 1.0)],
            effects: Some(vec![blur, text, fade]),
        };
        let (dur, _, _) = apply_and_probe(plan, 10.0).await.unwrap();
        assert!(
            dur > 2.7 && dur < 3.3,
            "expected ~3s with chained effects, got {dur}s"
        );
    }

    #[tokio::test]
    async fn integration_freeze_clip() {
        if !ffmpeg_ok() {
            return;
        }
        // Freeze clip at 2.0s for 2.0s duration
        let mut freeze = simple_clip(0.0, 2.0, 1.0);
        freeze.clip_type = Some("freeze".into());
        freeze.freeze_source_time = Some(2.0);
        freeze.freeze_duration = Some(2.0);

        // Concat with a normal clip so output has both
        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 2.0, 1.0), freeze],
            effects: None,
        };
        let (dur, _, _) = apply_and_probe(plan, 10.0).await.unwrap();
        // 2s normal + 2s freeze = 4s (freeze has no audio so audio may be shorter)
        assert!(dur > 3.5 && dur < 4.5, "expected ~4s, got {dur}s");
    }

    #[tokio::test]
    async fn integration_merge_replace_audio() {
        if !ffmpeg_ok() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let video = dir.path().join("v.mp4");
        let audio = dir.path().join("a.mp4");
        let output = dir.path().join("o.mp4");

        make_test_video(&video, 5.0).await.unwrap();
        make_test_video(&audio, 5.0).await.unwrap();

        let res = merge_audio_video(
            video.to_str().unwrap(),
            audio.to_str().unwrap(),
            output.to_str().unwrap(),
            true, // replace
            |_| {},
        )
        .await;
        assert!(res.is_ok(), "merge failed: {:?}", res.err());
        let meta = video_engine::probe_video(&output).await.unwrap();
        assert!(
            (meta.duration_seconds - 5.0).abs() < 0.5,
            "expected ~5s, got {}s",
            meta.duration_seconds
        );
    }

    #[tokio::test]
    async fn integration_merge_mix_audio() {
        if !ffmpeg_ok() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let video = dir.path().join("v.mp4");
        let audio = dir.path().join("a.mp4");
        let output = dir.path().join("o.mp4");

        make_test_video(&video, 5.0).await.unwrap();
        make_test_video(&audio, 5.0).await.unwrap();

        let res = merge_audio_video(
            video.to_str().unwrap(),
            audio.to_str().unwrap(),
            output.to_str().unwrap(),
            false, // mix
            |_| {},
        )
        .await;
        assert!(res.is_ok(), "merge mix failed: {:?}", res.err());
    }

    #[tokio::test]
    async fn integration_full_recomposition() {
        // End-to-end: apply_edits → merge_audio_video. Confirms the final
        // output reflects the edits and is muxed with the provided audio.
        if !ffmpeg_ok() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.mp4");
        let edited = dir.path().join("edited.mp4");
        let audio = dir.path().join("narration.mp4");
        let final_out = dir.path().join("final.mp4");

        make_test_video(&input, 10.0).await.unwrap();
        make_test_video(&audio, 4.0).await.unwrap();

        // Edit: concat two trimmed segments + one spotlight effect
        let mut fx = base_effect("spotlight", 0.5, 2.0);
        fx.spotlight = Some(SpotlightData {
            x: 0.5,
            y: 0.5,
            radius: 0.2,
            dim_opacity: 0.5,
        });
        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 2.0, 1.0), simple_clip(3.0, 5.0, 1.0)],
            effects: Some(vec![fx]),
        };

        apply_edits(
            input.to_str().unwrap(),
            edited.to_str().unwrap(),
            &plan,
            |_| {},
        )
        .await
        .unwrap();

        let edited_meta = video_engine::probe_video(&edited).await.unwrap();
        assert!(
            (edited_meta.duration_seconds - 4.0).abs() < 0.5,
            "edited should be ~4s, got {}s",
            edited_meta.duration_seconds
        );

        // Replace audio with narration track
        merge_audio_video(
            edited.to_str().unwrap(),
            audio.to_str().unwrap(),
            final_out.to_str().unwrap(),
            true,
            |_| {},
        )
        .await
        .unwrap();

        let final_meta = video_engine::probe_video(&final_out).await.unwrap();
        assert!(
            (final_meta.duration_seconds - 4.0).abs() < 0.5,
            "final should be ~4s, got {}s",
            final_meta.duration_seconds
        );
    }

    // ── Resolution + codec preservation ───────────────────────────────

    #[tokio::test]
    async fn integration_resolution_preserved_simple_trim() {
        if !ffmpeg_ok() {
            return;
        }
        let plan = VideoEditPlan {
            clips: vec![simple_clip(2.0, 6.0, 1.0)],
            effects: None,
        };
        let (_, w, h) = apply_and_probe(plan, 10.0).await.unwrap();
        assert_eq!((w, h), (320, 240));
    }

    #[tokio::test]
    async fn integration_resolution_preserved_with_speed() {
        if !ffmpeg_ok() {
            return;
        }
        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 4.0, 2.0)],
            effects: None,
        };
        let (_, w, h) = apply_and_probe(plan, 10.0).await.unwrap();
        assert_eq!((w, h), (320, 240));
    }

    #[tokio::test]
    async fn integration_resolution_preserved_with_concat() {
        if !ffmpeg_ok() {
            return;
        }
        let plan = VideoEditPlan {
            clips: vec![
                simple_clip(0.0, 2.0, 1.0),
                simple_clip(3.0, 5.0, 2.0),
                simple_clip(6.0, 8.0, 1.0),
            ],
            effects: None,
        };
        let (_, w, h) = apply_and_probe(plan, 10.0).await.unwrap();
        assert_eq!((w, h), (320, 240));
    }

    #[tokio::test]
    async fn integration_resolution_preserved_with_effects() {
        if !ffmpeg_ok() {
            return;
        }
        let mut blur = base_effect("blur", 0.0, 3.0);
        blur.blur = Some(BlurData {
            x: 0.1,
            y: 0.1,
            width: 0.3,
            height: 0.3,
            radius: 10.0,
            invert: None,
        });
        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 3.0, 1.0)],
            effects: Some(vec![blur]),
        };
        let (_, w, h) = apply_and_probe(plan, 10.0).await.unwrap();
        assert_eq!((w, h), (320, 240));
    }

    #[tokio::test]
    async fn integration_resolution_preserved_with_zoom_pan() {
        if !ffmpeg_ok() {
            return;
        }
        let mut clip = simple_clip(0.0, 4.0, 1.0);
        clip.zoom_pan = Some(ZoomPanEffect {
            start_region: crate::models::ZoomRegion {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            end_region: crate::models::ZoomRegion {
                x: 0.25,
                y: 0.25,
                width: 0.5,
                height: 0.5,
            },
            easing: EasingPreset::Linear,
        });
        let plan = VideoEditPlan {
            clips: vec![clip],
            effects: None,
        };
        let (_, w, h) = apply_and_probe(plan, 10.0).await.unwrap();
        // Zoom filter scales back to original dimensions
        assert_eq!((w, h), (320, 240));
    }

    #[tokio::test]
    async fn integration_output_codec_is_h264() {
        if !ffmpeg_ok() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.mp4");
        let output = dir.path().join("output.mp4");
        make_test_video(&input, 5.0).await.unwrap();

        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 3.0, 2.0)],
            effects: None,
        };
        apply_edits(
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            &plan,
            |_| {},
        )
        .await
        .unwrap();

        let meta = video_engine::probe_video(&output).await.unwrap();
        assert_eq!(meta.codec, "h264", "expected h264, got {}", meta.codec);
    }

    #[tokio::test]
    async fn integration_quality_file_size_reasonable() {
        // Sanity check: CRF 12 on a simple trimmed video should produce a file
        // larger than stream-copy but still sensible (not wildly inflated).
        if !ffmpeg_ok() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.mp4");
        let output = dir.path().join("output.mp4");
        make_test_video(&input, 5.0).await.unwrap();

        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 3.0, 2.0)],
            effects: None,
        };
        apply_edits(
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            &plan,
            |_| {},
        )
        .await
        .unwrap();

        let size = tokio::fs::metadata(&output).await.unwrap().len();
        // A 1.5s 320x240 h264 clip should be at least a few hundred bytes
        // (valid container with headers) and less than 10MB (not wildly inflated)
        assert!(size > 500, "output suspiciously small: {size} bytes");
        assert!(size < 10_000_000, "output suspiciously large: {size} bytes");
    }

    // ── burn_subtitles ────────────────────────────────────────────────

    async fn write_test_srt(path: &Path, content: &str) {
        tokio::fs::write(path, content).await.unwrap();
    }

    #[tokio::test]
    async fn integration_burn_subtitles_basic() {
        if !ffmpeg_ok() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.mp4");
        let srt_path = dir.path().join("sub.srt");
        let output = dir.path().join("out.mp4");
        make_test_video(&input, 5.0).await.unwrap();
        write_test_srt(
            &srt_path,
            "1\n00:00:00,000 --> 00:00:02,000\nFirst line\n\n\
             2\n00:00:02,000 --> 00:00:04,000\nSecond line\n\n",
        )
        .await;

        let style = SubtitleStyle::default();
        let res = burn_subtitles(
            input.to_str().unwrap(),
            srt_path.to_str().unwrap(),
            output.to_str().unwrap(),
            &style,
            |_| {},
        )
        .await;
        assert!(res.is_ok(), "burn_subtitles failed: {:?}", res.err());
        let meta = video_engine::probe_video(&output).await.unwrap();
        assert!(
            (meta.duration_seconds - 5.0).abs() < 0.3,
            "expected ~5s, got {}",
            meta.duration_seconds
        );
        assert_eq!((meta.width, meta.height), (320, 240));
    }

    #[tokio::test]
    async fn integration_burn_subtitles_with_custom_style() {
        if !ffmpeg_ok() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.mp4");
        let srt_path = dir.path().join("sub.srt");
        let output = dir.path().join("out.mp4");
        make_test_video(&input, 3.0).await.unwrap();
        write_test_srt(
            &srt_path,
            "1\n00:00:00,000 --> 00:00:02,000\nStyled subtitle\n\n",
        )
        .await;

        let style = SubtitleStyle {
            font_size: 28,
            color: "#ffff00".into(),
            outline_color: "#000000".into(),
            outline: 3,
            position: "top".into(),
        };
        let res = burn_subtitles(
            input.to_str().unwrap(),
            srt_path.to_str().unwrap(),
            output.to_str().unwrap(),
            &style,
            |_| {},
        )
        .await;
        assert!(
            res.is_ok(),
            "burn_subtitles with style failed: {:?}",
            res.err()
        );
    }

    #[tokio::test]
    async fn integration_burn_subtitles_with_unicode() {
        if !ffmpeg_ok() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.mp4");
        let srt_path = dir.path().join("sub.srt");
        let output = dir.path().join("out.mp4");
        make_test_video(&input, 3.0).await.unwrap();
        write_test_srt(
            &srt_path,
            "1\n00:00:00,000 --> 00:00:02,000\n日本語字幕\n\n",
        )
        .await;

        let style = SubtitleStyle::default();
        let res = burn_subtitles(
            input.to_str().unwrap(),
            srt_path.to_str().unwrap(),
            output.to_str().unwrap(),
            &style,
            |_| {},
        )
        .await;
        assert!(res.is_ok(), "unicode SRT should work: {:?}", res.err());
    }

    // ── extract_edit_thumbnails ──────────────────────────────────────

    #[tokio::test]
    async fn integration_extract_edit_thumbnails_count() {
        if !ffmpeg_ok() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.mp4");
        let out_dir = dir.path().join("thumbs");
        std::fs::create_dir_all(&out_dir).unwrap();
        make_test_video(&input, 10.0).await.unwrap();

        let thumbs = extract_edit_thumbnails(input.to_str().unwrap(), out_dir.to_str().unwrap(), 5)
            .await
            .unwrap();
        assert_eq!(
            thumbs.len(),
            5,
            "expected 5 thumbnails, got {}",
            thumbs.len()
        );
        // Each thumbnail should exist and be non-empty
        for t in &thumbs {
            let size = tokio::fs::metadata(t).await.unwrap().len();
            assert!(size > 100, "thumbnail {t} too small: {size} bytes");
        }
    }

    #[tokio::test]
    async fn integration_extract_edit_thumbnails_cache_hit() {
        if !ffmpeg_ok() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.mp4");
        let out_dir = dir.path().join("thumbs_cache");
        std::fs::create_dir_all(&out_dir).unwrap();
        make_test_video(&input, 5.0).await.unwrap();

        // First call populates the cache
        let first_start = std::time::Instant::now();
        let thumbs1 =
            extract_edit_thumbnails(input.to_str().unwrap(), out_dir.to_str().unwrap(), 5)
                .await
                .unwrap();
        let first_elapsed = first_start.elapsed();
        assert_eq!(thumbs1.len(), 5);

        // Second call with same inputs should hit cache and be substantially faster
        let second_start = std::time::Instant::now();
        let thumbs2 =
            extract_edit_thumbnails(input.to_str().unwrap(), out_dir.to_str().unwrap(), 5)
                .await
                .unwrap();
        let second_elapsed = second_start.elapsed();
        assert_eq!(thumbs2.len(), 5);
        // Cache hit should be at least 5x faster than the ffmpeg run
        assert!(
            second_elapsed * 5 < first_elapsed,
            "expected cache hit (2nd={:?}) to be much faster than cold (1st={:?})",
            second_elapsed,
            first_elapsed
        );
    }

    #[tokio::test]
    async fn integration_extract_single_frame() {
        if !ffmpeg_ok() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.mp4");
        let out = dir.path().join("frame.jpg");
        make_test_video(&input, 10.0).await.unwrap();

        let result =
            extract_single_frame(input.to_str().unwrap(), 5.0, out.to_str().unwrap()).await;
        assert!(
            result.is_ok(),
            "extract_single_frame failed: {:?}",
            result.err()
        );
        let size = tokio::fs::metadata(&out).await.unwrap().len();
        assert!(size > 100, "frame file too small: {size} bytes");
    }

    // ── End-to-end lossless pipeline ─────────────────────────────────

    /// Walk the full export pipeline: apply_edits → merge_audio_video →
    /// burn_subtitles. Confirm every edit propagates through and the final
    /// output's resolution matches the source.
    #[tokio::test]
    async fn integration_e2e_edits_propagate_to_export() {
        if !ffmpeg_ok() || !drawtext_available() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.mp4");
        let edited = dir.path().join("edited.mp4");
        let audio = dir.path().join("narration.mp4");
        let merged = dir.path().join("merged.mp4");
        let srt = dir.path().join("subs.srt");
        let final_out = dir.path().join("final.mp4");

        make_test_video(&input, 10.0).await.unwrap();
        make_test_video(&audio, 6.0).await.unwrap();
        tokio::fs::write(
            &srt,
            "1\n00:00:00,000 --> 00:00:02,000\nOpening line\n\n\
             2\n00:00:03,000 --> 00:00:05,000\n日本語字幕\n\n",
        )
        .await
        .unwrap();

        // Step 1: edit with trim + speed + zoom-pan + spotlight + text
        let mut clip1 = simple_clip(0.0, 3.0, 1.0);
        clip1.zoom_pan = Some(ZoomPanEffect {
            start_region: crate::models::ZoomRegion {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            end_region: crate::models::ZoomRegion {
                x: 0.3,
                y: 0.3,
                width: 0.4,
                height: 0.4,
            },
            easing: EasingPreset::EaseInOut,
        });
        let clip2 = simple_clip(4.0, 7.0, 2.0); // 3s source → 1.5s output
                                                // Output timeline: clip1 = 3s, clip2 = 1.5s → total 4.5s
        let mut spotlight = base_effect("spotlight", 0.5, 2.0);
        spotlight.spotlight = Some(SpotlightData {
            x: 0.5,
            y: 0.5,
            radius: 0.2,
            dim_opacity: 0.6,
        });
        let mut text = base_effect("text", 3.0, 4.5);
        text.text = Some(TextData {
            content: "End of clip".into(),
            x: 0.5,
            y: 0.9,
            font_size: 5.0,
            color: "#ffffff".into(),
            font_family: None,
            bold: Some(true),
            italic: None,
            underline: None,
            background: None,
            align: None,
            opacity: Some(1.0),
        });

        let plan = VideoEditPlan {
            clips: vec![clip1, clip2],
            effects: Some(vec![spotlight, text]),
        };
        apply_edits(
            input.to_str().unwrap(),
            edited.to_str().unwrap(),
            &plan,
            |_| {},
        )
        .await
        .unwrap();

        let edited_meta = video_engine::probe_video(&edited).await.unwrap();
        // Expected: 3s + 1.5s = 4.5s (±0.5 for encode jitter)
        assert!(
            (edited_meta.duration_seconds - 4.5).abs() < 0.6,
            "edited duration drift: expected ~4.5s, got {}",
            edited_meta.duration_seconds
        );
        assert_eq!(
            (edited_meta.width, edited_meta.height),
            (320, 240),
            "resolution must be preserved through apply_edits"
        );

        // Step 2: merge with narration audio (replace original)
        merge_audio_video(
            edited.to_str().unwrap(),
            audio.to_str().unwrap(),
            merged.to_str().unwrap(),
            true,
            |_| {},
        )
        .await
        .unwrap();

        let merged_meta = video_engine::probe_video(&merged).await.unwrap();
        // Merge preserves video via -c:v copy → duration & resolution unchanged
        assert!(
            (merged_meta.duration_seconds - edited_meta.duration_seconds).abs() < 0.3,
            "merge changed duration: was {}, now {}",
            edited_meta.duration_seconds,
            merged_meta.duration_seconds
        );
        assert_eq!(
            (merged_meta.width, merged_meta.height),
            (edited_meta.width, edited_meta.height),
            "merge must preserve resolution"
        );

        // Step 3: burn subtitles
        let style = SubtitleStyle::default();
        burn_subtitles(
            merged.to_str().unwrap(),
            srt.to_str().unwrap(),
            final_out.to_str().unwrap(),
            &style,
            |_| {},
        )
        .await
        .unwrap();

        let final_meta = video_engine::probe_video(&final_out).await.unwrap();
        assert!(
            (final_meta.duration_seconds - merged_meta.duration_seconds).abs() < 0.3,
            "burn_subtitles changed duration: was {}, now {}",
            merged_meta.duration_seconds,
            final_meta.duration_seconds
        );
        assert_eq!(
            (final_meta.width, final_meta.height),
            (merged_meta.width, merged_meta.height),
            "burn_subtitles must preserve resolution"
        );
        assert_eq!(final_meta.codec, "h264", "final output must be h264");
    }

    /// Regression guard: a single clip with speed=1 and an overlay effect
    /// (blur/spotlight/fade) must still run the effects pass — the fast-path
    /// code previously skipped effects, silently producing an unedited output.
    #[tokio::test]
    async fn integration_single_clip_with_overlay_effect_not_dropped() {
        if !ffmpeg_ok() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.mp4");
        let output = dir.path().join("out.mp4");
        make_test_video(&input, 5.0).await.unwrap();

        // Source size before any edit
        let src_size = tokio::fs::metadata(&input).await.unwrap().len();

        // Single clip, speed=1, no zoom/freeze, but WITH a fade effect.
        // Before the fix, this went through the fast path and effects were
        // silently dropped.
        let mut fade = base_effect("fade", 0.0, 5.0);
        fade.fade = Some(FadeData {
            color: "#000000".into(),
            opacity: 0.5,
        });
        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 5.0, 1.0)],
            effects: Some(vec![fade]),
        };
        apply_edits(
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            &plan,
            |_| {},
        )
        .await
        .unwrap();

        // If the fast path ran, it would be `-c copy` so size ≈ src.
        // With CRF 0 + fade drawbox overlay, size should be different
        // (either larger due to the CRF 0 re-encode, or same if encoder is
        // very efficient). The key check: the output actually went through
        // the effects pass. We can't easily detect the overlay visually in
        // a unit test, but we can probe that the output exists and isn't a
        // byte-for-byte copy of the input.
        let out_size = tokio::fs::metadata(&output).await.unwrap().len();
        assert!(out_size > 0);
        // The CRF-0 lossless re-encode will be significantly larger than
        // a CRF-36 testsrc input; if sizes are nearly identical something
        // copied the source without processing.
        let ratio = out_size as f64 / src_size as f64;
        assert!(
            ratio > 2.0,
            "output/input size ratio = {ratio:.2} — suggests fast path ran and skipped effects"
        );
    }

    /// Lossless verification: when a clip has `speed=1.0`, no zoom, no fps
    /// override, no effects, the output's video bitrate should be at least
    /// as high as the source (CRF 0 = bit-exact). This is a weak check (same
    /// decode but ffprobe counts both bitrates, good enough as a regression
    /// guard against accidentally re-introducing CRF > 0).
    #[tokio::test]
    async fn integration_lossless_encode_simple_trim() {
        if !ffmpeg_ok() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.mp4");
        let output = dir.path().join("out.mp4");
        make_test_video(&input, 5.0).await.unwrap();

        // Full-source single clip with speed=1 falls through the fast path
        // (stream-copy). But for multi-clip we re-encode at CRF 0 — so use
        // two clips to force the re-encode path.
        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 2.5, 1.0), simple_clip(2.5, 5.0, 1.0)],
            effects: None,
        };
        apply_edits(
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            &plan,
            |_| {},
        )
        .await
        .unwrap();

        let src_size = tokio::fs::metadata(&input).await.unwrap().len();
        let out_size = tokio::fs::metadata(&output).await.unwrap().len();
        // Lossless output from testsrc should be AT LEAST as large as the
        // heavily-compressed source. A CRF-0 encode cannot produce a smaller
        // file than the lossy input unless something is stripped.
        assert!(
            out_size >= src_size,
            "lossless output ({out_size}) unexpectedly smaller than source ({src_size}) — \
             likely indicates we're not actually encoding lossless"
        );
    }
}
