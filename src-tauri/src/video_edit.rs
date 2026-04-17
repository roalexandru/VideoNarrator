//! Video editing operations: trim, speed, frame dropping, zoom/pan, freeze frame, and concatenation.

use crate::error::NarratorError;
use crate::models::{EasingPreset, ZoomPanEffect};
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
                return Err(NarratorError::ExportError(format!(
                    "Path not allowed: {p}"
                )));
            }
        }
    }

    Ok(path)
}

/// Validate clip parameters (S2: DoS, F4: bounds).
fn validate_clip(clip: &EditClip, duration: f64, index: usize) -> Result<(), NarratorError> {
    let err = |msg: &str| NarratorError::ExportError(format!("Clip {index}: {msg}"));
    if clip.speed <= 0.0 || clip.speed > 100.0 {
        return Err(err(&format!("speed {} out of range (0, 100]", clip.speed)));
    }
    if clip.start_seconds < -0.1 {
        return Err(err("start_seconds is negative"));
    }
    if clip.end_seconds < clip.start_seconds {
        return Err(err("end_seconds < start_seconds"));
    }
    if clip.end_seconds > duration + 1.0 {
        return Err(err("end_seconds exceeds video duration"));
    }
    if let Some(fd) = clip.freeze_duration {
        if fd <= 0.0 || fd > 600.0 {
            return Err(err(&format!(
                "freeze_duration {} out of range (0, 600]",
                fd
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

/// Escape text for ffmpeg drawtext filter (S3: injection prevention).
/// Will be used when text overlay rendering is implemented.
#[allow(dead_code)]
pub fn escape_ffmpeg_text(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\'', "'\\''")
        .replace(':', "\\:")
        .replace('%', "%%")
        .replace(['\n', '\r'], "")
}

/// Validate a hex color string (S3: injection prevention).
/// Will be used when overlay effect rendering is implemented.
#[allow(dead_code)]
pub fn validate_hex_color(s: &str) -> Result<String, NarratorError> {
    let trimmed = s.trim().trim_start_matches('#');
    if (trimmed.len() == 6 || trimmed.len() == 8) && trimmed.chars().all(|c| c.is_ascii_hexdigit())
    {
        Ok(format!("#{trimmed}"))
    } else {
        Err(NarratorError::ExportError(format!(
            "Invalid hex color: {s}"
        )))
    }
}

const MAX_OUTPUT_FPS: f64 = 60.0;

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
    pub background: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FadeData {
    pub color: String,
    pub opacity: f64,
}

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
    let mut cmd = Command::new(ffmpeg.as_os_str());
    cmd.no_window()
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| NarratorError::FfmpegFailed(format!("Failed to start ffmpeg: {e}")))?;

    // Read stderr line by line for progress
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
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|e| NarratorError::FfmpegFailed(format!("ffmpeg process error: {e}")))?;

    if !status.success() {
        return Err(NarratorError::FfmpegFailed(format!(
            "ffmpeg exited with status {}",
            status
        )));
    }

    on_progress(100.0);
    Ok(())
}

/// Extract the time= value from an ffmpeg stderr line.
fn extract_time_from_ffmpeg_line(line: &str) -> Option<String> {
    // ffmpeg progress lines contain "time=HH:MM:SS.mm" or "time=N/A"
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

/// Process a freeze frame clip: extract a single frame, then create a video of it held for the specified duration.
async fn process_freeze_clip(
    ffmpeg: &Path,
    input_path: &str,
    clip: &EditClip,
    clip_index: usize,
    out_dir: &Path,
    meta: &crate::models::VideoMetadata,
) -> Result<PathBuf, NarratorError> {
    let width = meta.width;
    let height = meta.height;
    let fps = meta.fps;
    let timestamp = clip.freeze_source_time.unwrap_or(clip.start_seconds);
    let duration = clip.freeze_duration.unwrap_or(3.0);
    let frame_path = out_dir.join(format!("_freeze_frame_{:03}.jpg", clip_index));
    let clip_path = out_dir.join(format!("_edit_clip_{:03}.mp4", clip_index));

    // Step 1: Extract the single frame
    let output = Command::new(ffmpeg.as_os_str())
        .no_window()
        .args([
            "-y",
            "-ss",
            &format!("{:.3}", timestamp),
            "-i",
            input_path,
            "-frames:v",
            "1",
            "-q:v",
            "2",
        ])
        .arg(frame_path.as_os_str())
        .output()
        .await
        .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(NarratorError::FfmpegFailed(format!(
            "Freeze frame extraction failed: {}",
            &stderr[stderr.len().saturating_sub(500)..]
        )));
    }

    // Step 2: Create a video from the still frame
    let mut args: Vec<String> = vec![
        "-y".into(),
        "-loop".into(),
        "1".into(),
        "-i".into(),
        frame_path.to_string_lossy().to_string(),
        "-t".into(),
        format!("{:.3}", duration),
    ];

    // If zoom/pan is specified, apply zoompan filter to the still image
    if let Some(ref zp) = clip.zoom_pan {
        let zp_filter = build_zoompan_filter(zp, width, height, fps, duration);
        args.extend([
            "-vf".into(),
            format!("{},scale={}:{}", zp_filter, width, height),
        ]);
    } else {
        args.extend(["-vf".into(), format!("scale={}:{}", width, height)]);
    }

    args.extend([
        "-c:v".into(),
        "libx264".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-r".into(),
        format!("{:.0}", fps.min(MAX_OUTPUT_FPS)),
        "-an".into(), // no audio for freeze frames
    ]);
    args.push(clip_path.to_string_lossy().to_string());

    let output = Command::new(ffmpeg.as_os_str())
        .no_window()
        .args(&args)
        .output()
        .await
        .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(NarratorError::FfmpegFailed(format!(
            "Freeze clip creation failed: {}",
            &stderr[stderr.len().saturating_sub(500)..]
        )));
    }

    // Clean up extracted frame
    let _ = tokio::fs::remove_file(&frame_path).await;

    Ok(clip_path)
}

/// Build an ffmpeg zoompan filter expression from a ZoomPanEffect.
///
/// The zoompan filter uses per-frame expressions for z (zoom), x (pan-x), y (pan-y).
/// `on` = current frame number, `d` = total frames (set as the duration parameter).
fn build_zoompan_filter(
    effect: &ZoomPanEffect,
    width: u32,
    height: u32,
    fps: f64,
    duration_seconds: f64,
) -> String {
    let total_frames = (duration_seconds * fps.min(MAX_OUTPUT_FPS))
        .round()
        .max(1.0) as u32;

    // Zoom: 1/region_width gives the zoom factor (region covering 50% width = 2x zoom)
    let z_start = 1.0 / effect.start_region.width.max(0.01);
    let z_end = 1.0 / effect.end_region.width.max(0.01);

    // Pan centers (normalized 0-1)
    let sx = effect.start_region.x + effect.start_region.width / 2.0;
    let sy = effect.start_region.y + effect.start_region.height / 2.0;
    let ex = effect.end_region.x + effect.end_region.width / 2.0;
    let ey = effect.end_region.y + effect.end_region.height / 2.0;

    // Easing expression for progress (on/d mapped through easing function)
    let progress = match effect.easing {
        EasingPreset::Linear => "on/d".to_string(),
        EasingPreset::EaseIn => "(on/d)*(on/d)".to_string(),
        EasingPreset::EaseOut => "(on/d)*(2-on/d)".to_string(),
        EasingPreset::EaseInOut => {
            "if(lt(on/d,0.5),2*(on/d)*(on/d),-1+(4-2*(on/d))*(on/d))".to_string()
        }
    };

    format!(
        "zoompan=z='{z_s}+({z_e}-{z_s})*({p})':x='iw*({sx}+({ex}-{sx})*({p}))-iw/zoom/2':y='ih*({sy}+({ey}-{sy})*({p}))-ih/zoom/2':d={d}:s={w}x{h}:fps={fps}",
        z_s = z_start,
        z_e = z_end,
        p = progress,
        sx = sx,
        ex = ex,
        sy = sy,
        ey = ey,
        d = total_frames,
        w = width,
        h = height,
        fps = fps.min(MAX_OUTPUT_FPS).round() as u32,
    )
}

/// Build a crop+scale filter for zoom/pan on VIDEO clips (not stills).
/// Uses ffmpeg's `crop` filter with expression-based animated parameters
/// and `n` (frame number) for interpolation. This avoids the zoompan filter
/// issues with video input (zoompan is designed for still images).
/// Build a crop+scale filter for zoom/pan on VIDEO clips.
/// Uses ffmpeg's `crop` filter with expression-based animated parameters.
/// All values are clamped to valid ranges using min/max to prevent ffmpeg errors.
fn build_zoompan_filter_for_video(
    effect: &ZoomPanEffect,
    width: u32,
    height: u32,
    total_frames: f64,
) -> String {
    let w = width as f64;
    let h = height as f64;

    let sx = effect.start_region.x.clamp(0.0, 0.99);
    let sy = effect.start_region.y.clamp(0.0, 0.99);
    let sw = effect.start_region.width.clamp(0.05, 1.0);
    let sh = effect.start_region.height.clamp(0.05, 1.0);
    let ex = effect.end_region.x.clamp(0.0, 0.99);
    let ey = effect.end_region.y.clamp(0.0, 0.99);
    let ew = effect.end_region.width.clamp(0.05, 1.0);
    let eh = effect.end_region.height.clamp(0.05, 1.0);

    let tf = total_frames.max(1.0);

    let progress = match effect.easing {
        EasingPreset::Linear => format!("min(n/{tf},1)"),
        EasingPreset::EaseIn => format!("min(n/{tf},1)*min(n/{tf},1)"),
        EasingPreset::EaseOut => format!("min(n/{tf},1)*(2-min(n/{tf},1))"),
        EasingPreset::EaseInOut => format!(
            "if(lt(n/{tf},0.5),2*min(n/{tf},1)*min(n/{tf},1),-1+(4-2*min(n/{tf},1))*min(n/{tf},1))"
        ),
    };

    // Crop dimensions — clamped to at least 2px and at most iw/ih
    let crop_w = format!("max(2,min(iw,({sw}+({ew}-{sw})*({progress}))*{w}))");
    let crop_h = format!("max(2,min(ih,({sh}+({eh}-{sh})*({progress}))*{h}))");
    // Crop position — clamped so crop doesn't extend past frame edge
    let crop_x = format!("max(0,min(iw-out_w,({sx}+({ex}-{sx})*({progress}))*{w}))");
    let crop_y = format!("max(0,min(ih-out_h,({sy}+({ey}-{sy})*({progress}))*{h}))");

    // Ensure even dimensions for h264 compatibility
    format!(
        "crop='{crop_w}':'{crop_h}':'{crop_x}':'{crop_y}',scale={width}:{height}:flags=lanczos,setsar=1"
    )
}

pub async fn apply_edits(
    input_path: &str,
    output_path: &str,
    plan: &VideoEditPlan,
    on_progress: impl Fn(f64),
) -> Result<String, NarratorError> {
    // S1: Validate paths
    validate_path(input_path)?;
    validate_path(output_path)?;

    let ffmpeg = video_engine::detect_ffmpeg()?;
    let out_dir = Path::new(output_path).parent().unwrap_or(Path::new("/tmp"));
    let total = plan.clips.len();

    if total == 0 {
        return Err(NarratorError::ExportError("No clips to process".into()));
    }

    // Probe video metadata (needed for freeze frame and zoom/pan)
    let meta = video_engine::probe_video(std::path::Path::new(input_path)).await?;

    // S2/F4: Validate all clips
    for (i, clip) in plan.clips.iter().enumerate() {
        validate_clip(clip, meta.duration_seconds, i)?;
        if let Some(ref zp) = clip.zoom_pan {
            validate_zoom(zp)?;
        }
    }

    // If single clip with no modifications, check if it covers the full source
    let has_effects =
        plan.clips[0].clip_type.as_deref() == Some("freeze") || plan.clips[0].zoom_pan.is_some();
    if total == 1
        && plan.clips[0].speed == 1.0
        && plan.clips[0].fps_override.is_none()
        && !has_effects
    {
        let clip = &plan.clips[0];

        // Check if the clip covers the full video (using already-probed metadata)
        let covers_full =
            clip.start_seconds < 0.5 && (clip.end_seconds - meta.duration_seconds).abs() < 0.5;

        if covers_full {
            // No edits — just use the original file directly (symlink or copy)
            if input_path != output_path {
                tokio::fs::copy(input_path, output_path).await?;
            }
            on_progress(100.0);
            return Ok(output_path.to_string());
        }

        // Trimmed single clip — use accurate seek (input seeking + output duration)
        let duration = clip.end_seconds - clip.start_seconds;
        let output = Command::new(ffmpeg.as_os_str())
            .no_window()
            .args([
                "-y",
                "-ss",
                &format!("{:.3}", clip.start_seconds),
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
        return Ok(output_path.to_string());
    }

    // Process each clip
    let mut clip_files: Vec<PathBuf> = Vec::new();

    for (i, clip) in plan.clips.iter().enumerate() {
        on_progress((i as f64 / total as f64) * 80.0);

        // Handle freeze frame clips separately
        if clip.clip_type.as_deref() == Some("freeze") {
            let clip_path =
                process_freeze_clip(&ffmpeg, input_path, clip, i, out_dir, &meta).await?;
            clip_files.push(clip_path);
            continue;
        }

        let clip_path = out_dir.join(format!("_edit_clip_{:03}.mp4", i));
        let mut args: Vec<String> = vec!["-y".into(), "-i".into(), input_path.into()];

        // Trim
        args.extend(["-ss".into(), format!("{:.3}", clip.start_seconds)]);
        args.extend(["-to".into(), format!("{:.3}", clip.end_seconds)]);

        // Build video filter chain
        let mut vfilters = Vec::new();
        let mut afilters = Vec::new();
        let needs_speed = (clip.speed - 1.0).abs() > 0.01;
        let has_zoom = clip.zoom_pan.is_some();

        // Zoom/Pan effect — animated crop+scale for video clips
        if let Some(ref zp) = clip.zoom_pan {
            let clip_duration = clip.end_seconds - clip.start_seconds;
            let total_frames = (clip_duration * meta.fps.min(MAX_OUTPUT_FPS)).round().max(1.0);
            let zp_filter = build_zoompan_filter_for_video(
                zp, meta.width, meta.height, total_frames,
            );
            vfilters.push(zp_filter);
        }

        if let Some(fps) = clip.fps_override {
            vfilters.push(format!("fps={:.3}", fps));
        }

        if needs_speed {
            if clip.skip_frames {
                let n = clip.speed.round().max(2.0) as u32;
                vfilters.push(format!("select='not(mod(n\\,{}))'", n));
                vfilters.push("setpts=N/FRAME_RATE/TB".to_string());
            } else {
                vfilters.push(format!("setpts={:.4}*PTS", 1.0 / clip.speed));
            }
        }

        // Audio filter handling (actual -an or -c:a is added below with the encoder args)
        let drop_audio = needs_speed && clip.skip_frames;
        if needs_speed && !clip.skip_frames {
            let mut atempo_chain = Vec::new();
            let mut remaining = clip.speed;
            while remaining < 0.5 {
                atempo_chain.push("atempo=0.5".to_string());
                remaining /= 0.5;
            }
            atempo_chain.push(format!("atempo={:.4}", remaining));
            afilters = atempo_chain;
        }

        // Always re-encode every clip with identical settings for reliable concat.
        // Stream copy mixing causes "no streams" errors due to format mismatches.
        vfilters.push("format=yuv420p".to_string());
        args.extend(["-vf".into(), vfilters.join(",")]);
        args.extend([
            "-c:v".into(), "libx264".into(),
            "-preset".into(), "medium".into(),
            "-crf".into(), "15".into(),
        ]);
        if !afilters.is_empty() {
            args.extend(["-af".into(), afilters.join(",")]);
        }
        if !drop_audio {
            args.extend(["-c:a".into(), "aac".into(), "-b:a".into(), "256k".into()]);
        } else {
            args.extend(["-an".into()]);
        }
        args.extend(["-movflags".into(), "+faststart".into()]);
        args.push(clip_path.to_string_lossy().to_string());

        tracing::info!("Clip {i}: zoom={has_zoom} speed={} filters={}", clip.speed, vfilters.len());

        let output = Command::new(ffmpeg.as_os_str())
            .no_window()
            .args(&args)
            .output()
            .await
            .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            tracing::error!("Clip {i} ffmpeg args: {:?}", &args);
            tracing::error!("Clip {i} stderr: {stderr}");
            tracing::error!("Clip {i} stdout: {stdout}");
            // Get the last meaningful part of stderr (skip the banner)
            let err_tail = stderr[stderr.len().saturating_sub(800)..].trim();
            // Find lines with actual errors (not the banner)
            let meaningful: String = err_tail
                .lines()
                .filter(|l| {
                    let ll = l.to_lowercase();
                    ll.contains("error") || ll.contains("invalid") || ll.contains("no such")
                        || ll.contains("not found") || ll.contains("failed") || ll.contains("unknown")
                        || ll.contains("unrecognized") || ll.contains("does not")
                })
                .collect::<Vec<_>>()
                .join("; ");
            let detail = if meaningful.is_empty() { err_tail.to_string() } else { meaningful };
            return Err(NarratorError::FfmpegFailed(format!(
                "Clip {i} failed: {detail}"
            )));
        }

        // Verify clip file was actually created and has content
        let clip_size = tokio::fs::metadata(&clip_path).await.map(|m| m.len()).unwrap_or(0);
        if clip_size == 0 {
            tracing::error!("Clip {i} produced empty file: {}", clip_path.display());
            tracing::error!("Clip {i} ffmpeg args: {:?}", &args);
            return Err(NarratorError::FfmpegFailed(format!(
                "Clip {i} produced empty output. Try removing zoom/pan effects or simplifying edits on this clip."
            )));
        }
        tracing::info!("Clip {i} OK: {} bytes", clip_size);
        clip_files.push(clip_path);
    }

    on_progress(85.0);

    // Concat all clips — all clips are re-encoded with identical h264 settings
    if clip_files.len() == 1 {
        tokio::fs::rename(&clip_files[0], output_path).await?;
    } else {
        let concat_list = out_dir.join("_edit_concat.txt");
        let list_content: String = clip_files
            .iter()
            .map(|p| {
                let escaped = p
                    .to_string_lossy()
                    .replace('\\', "\\\\")
                    .replace(['\n', '\r'], "")
                    .replace('\'', "'\\''");
                format!("file '{}'", escaped)
            })
            .collect::<Vec<_>>()
            .join("\n");
        tokio::fs::write(&concat_list, &list_content).await?;

        tracing::info!("Concat: {} clips", clip_files.len());

        // All clips are identically encoded (h264/aac), so stream-copy concat should work
        let output = Command::new(ffmpeg.as_os_str())
            .no_window()
            .args(["-y", "-f", "concat", "-safe", "0", "-i"])
            .arg(concat_list.as_os_str())
            .args(["-c", "copy", "-movflags", "+faststart", output_path])
            .output()
            .await
            .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;

        if !output.status.success() {
            tracing::warn!("Stream-copy concat failed, falling back to re-encode");
            let output2 = Command::new(ffmpeg.as_os_str())
                .no_window()
                .args(["-y", "-f", "concat", "-safe", "0", "-i"])
                .arg(concat_list.as_os_str())
                .args([
                    "-c:v",
                    "libx264",
                    "-preset",
                    "medium",
                    "-crf",
                    "15",
                    "-pix_fmt",
                    "yuv420p",
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
                .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;

            if !output2.status.success() {
                let stderr = String::from_utf8_lossy(&output2.stderr);
                return Err(NarratorError::FfmpegFailed(format!(
                    "Concat failed: {}",
                    &stderr[stderr.len().saturating_sub(500)..]
                )));
            }
        }

        let _ = tokio::fs::remove_file(&concat_list).await;
    }

    // Cleanup temp clips
    for p in &clip_files {
        let _ = tokio::fs::remove_file(p).await;
    }

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

    // Mix original + narration audio (amix) — re-encodes audio, so use progress reporting.
    let meta = video_engine::probe_video(Path::new(video_path)).await?;
    let total_duration = meta.duration_seconds;

    let filter = "[0:a][1:a]amix=inputs=2:duration=first:normalize=1[a]";
    let result = run_ffmpeg_with_progress(
        &ffmpeg,
        &[
            "-y",
            "-i",
            video_path,
            "-i",
            audio_path,
            "-filter_complex",
            filter,
            "-map",
            "0:v",
            "-map",
            "[a]",
            "-c:v",
            "copy",
            "-c:a",
            "aac",
            output_path,
        ],
        total_duration,
        &on_progress,
    )
    .await;

    if let Err(_e) = result {
        // Fallback: video might not have audio stream, use narration audio only
        tracing::warn!("amix failed, trying narration-only fallback");
        let fallback = Command::new(ffmpeg.as_os_str())
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

        if !fallback.status.success() {
            let stderr = String::from_utf8_lossy(&fallback.stderr);
            return Err(NarratorError::FfmpegFailed(format!(
                "Audio merge failed: {}",
                &stderr[stderr.len().saturating_sub(500)..]
            )));
        }
    }

    on_progress(100.0);
    Ok(output_path.to_string())
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
            "medium",
            "-crf",
            "23",
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
                "medium",
                "-crf",
                "23",
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
}
