//! Video processing engine using ffmpeg for frame extraction and probing.

use crate::error::NarratorError;
use crate::ffmpeg_progress::{extract_time_from_ffmpeg_line, parse_ffmpeg_time};
use crate::models::{Frame, FrameConfig, VideoMetadata};
use crate::process_utils::CommandNoWindow;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
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

    let duration = resolve_video_duration(video_stream, &json["format"]);

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

/// True when `path` has at least one audio stream. Used by the mix path
/// to take the narration-only fallback proactively, instead of relying on
/// English-only ffmpeg stderr string-matching after a failed mix.
///
/// Returns `Ok(false)` on any file that ffprobe parses but lists no audio
/// stream. Returns `Err` only when ffprobe itself fails — in that case the
/// caller should propagate rather than silently fall back.
pub async fn probe_has_audio_stream(path: &Path) -> Result<bool, NarratorError> {
    let ffprobe = detect_ffprobe()?;
    let output = Command::new(ffprobe.as_os_str())
        .no_window()
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_streams",
            "-select_streams",
            "a",
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

    Ok(json["streams"]
        .as_array()
        .map(|streams| !streams.is_empty())
        .unwrap_or(false))
}

/// Probe the pixel format of the first video stream. Used by the overflow
/// padding path to decide whether a libx264 re-encode would silently
/// downgrade the source's colour pipeline (e.g. 10-bit → 8-bit).
///
/// Returns `Ok(None)` when ffprobe parsed the file but found no video
/// stream or no pix_fmt field (e.g. image containers). Returns `Err` only
/// when ffprobe itself failed to run.
pub async fn probe_pix_fmt(path: &Path) -> Result<Option<String>, NarratorError> {
    let ffprobe = detect_ffprobe()?;
    let output = Command::new(ffprobe.as_os_str())
        .no_window()
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_streams",
            "-select_streams",
            "v:0",
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

    Ok(json["streams"][0]["pix_fmt"]
        .as_str()
        .map(|s| s.to_string()))
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

/// Resolve the authoritative video duration from an ffprobe JSON blob.
///
/// Prefers the video stream's own `duration` over the container
/// `format.duration`. The format value is the max across all streams, so a
/// trailing audio track that outlives the picture (e.g. a previously-narrated
/// Narrator export whose audio holds the last frame) would otherwise overstate
/// visual length and mislead narration generation into emitting segments past
/// the end of the video. Falls back to format duration when the stream omits
/// its own (some containers like WebM do).
fn resolve_video_duration(video_stream: &serde_json::Value, format: &serde_json::Value) -> f64 {
    let stream_duration = video_stream["duration"]
        .as_str()
        .and_then(|d| d.parse::<f64>().ok())
        .filter(|d| d.is_finite() && *d > 0.0);
    let format_duration = format["duration"]
        .as_str()
        .and_then(|d| d.parse::<f64>().ok())
        .filter(|d| d.is_finite() && *d > 0.0);
    stream_duration.or(format_duration).unwrap_or(0.0)
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
///
/// Sampling strategy: when the video has an audio stream we first try to
/// anchor frame extraction on scene changes + silence boundaries, which
/// lines up better with meaningful visual events than a fixed interval.
/// If that yields too few anchors (static slideshows, screencasts, very
/// short clips) we fall back to the fixed-interval path that matches the
/// historical behaviour.
pub async fn extract_frames(
    video_path: &Path,
    config: &FrameConfig,
    output_dir: &Path,
    on_frame: impl Fn(Frame) + Send + Sync + 'static,
    on_tick: impl Fn(f64, String) + Send + Sync + 'static,
    cancel_flag: Option<Arc<AtomicBool>>,
) -> Result<Vec<Frame>, NarratorError> {
    let ffmpeg = detect_ffmpeg()?;

    // Ensure output dir exists
    tokio::fs::create_dir_all(output_dir)
        .await
        .map_err(|e| NarratorError::FrameExtractionError(e.to_string()))?;

    let metadata = probe_video(video_path).await?;

    let on_frame: Arc<dyn Fn(Frame) + Send + Sync> = Arc::new(on_frame);
    let on_tick: Arc<dyn Fn(f64, String) + Send + Sync> = Arc::new(on_tick);

    // Attempt anchor-based sampling first; fall back to fixed-interval if it
    // didn't yield enough anchors or if any detection step errored. We need
    // at least `MIN_ANCHORS` frames for the fallback threshold to feel
    // meaningful — fewer than that and the LLM can't tell scene structure
    // from a handful of samples.
    const MIN_ANCHORS: usize = 3;
    if let Ok(anchors) =
        detect_anchors(&ffmpeg, video_path, &metadata, config, on_tick.clone()).await
    {
        if anchors.len() >= MIN_ANCHORS {
            return extract_frames_at_anchors(
                &ffmpeg,
                video_path,
                &anchors,
                &metadata,
                output_dir,
                on_frame,
                on_tick,
                cancel_flag,
            )
            .await;
        }
        tracing::info!(
            "anchor-based sampling found only {} frames (< {}), falling back to fixed interval",
            anchors.len(),
            MIN_ANCHORS
        );
    } else {
        tracing::warn!("anchor detection failed, falling back to fixed interval");
    }

    extract_frames_fixed_interval(
        &ffmpeg, video_path, &metadata, config, output_dir, on_frame, on_tick,
    )
    .await
}

/// Check an optional cancel flag and return `Cancelled` if set. Kept as a
/// small helper so the anchor loop doesn't sprout repeated `match` blocks.
fn check_cancelled(cancel_flag: &Option<Arc<AtomicBool>>) -> Result<(), NarratorError> {
    if let Some(flag) = cancel_flag.as_ref() {
        if flag.load(Ordering::SeqCst) {
            return Err(NarratorError::Cancelled);
        }
    }
    Ok(())
}

/// Parse `showinfo` stderr lines (`... pts_time:X.Y ...`) into timestamps.
pub(crate) fn parse_showinfo_timestamps(stderr: &str) -> Vec<f64> {
    let mut out = Vec::new();
    for line in stderr.lines() {
        // showinfo prefixes each frame with `[Parsed_showinfo_N @ ...]` and
        // the timestamp is reported as `pts_time:1.234`.
        if let Some(idx) = line.find("pts_time:") {
            let rest = &line[idx + "pts_time:".len()..];
            let end = rest
                .find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
                .unwrap_or(rest.len());
            if let Ok(t) = rest[..end].parse::<f64>() {
                if t.is_finite() && t >= 0.0 {
                    out.push(t);
                }
            }
        }
    }
    out
}

/// Parse `silencedetect` stderr lines (`silence_start:` / `silence_end:`)
/// into midpoint timestamps — those land in the quiet gap between phrases,
/// which is usually a good narration anchor because the frame there is
/// stable and shows the speaker finishing a thought.
pub(crate) fn parse_silence_midpoints(stderr: &str) -> Vec<f64> {
    let mut starts: Vec<f64> = Vec::new();
    let mut ends: Vec<f64> = Vec::new();
    for line in stderr.lines() {
        if let Some(idx) = line.find("silence_start: ") {
            let tail = &line[idx + "silence_start: ".len()..];
            let end = tail
                .find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
                .unwrap_or(tail.len());
            if let Ok(t) = tail[..end].parse::<f64>() {
                starts.push(t.max(0.0));
            }
        } else if let Some(idx) = line.find("silence_end: ") {
            let tail = &line[idx + "silence_end: ".len()..];
            let end = tail
                .find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
                .unwrap_or(tail.len());
            if let Ok(t) = tail[..end].parse::<f64>() {
                ends.push(t.max(0.0));
            }
        }
    }
    // Pair starts with ends positionally. A silencedetect pass always logs
    // starts before the matching end, but runs may finish with an unterminated
    // silence (ending at EOF) that has no `silence_end` line. Ignore unpaired
    // trailing starts.
    let pairs = starts.len().min(ends.len());
    (0..pairs)
        .map(|i| 0.5 * (starts[i] + ends[i]))
        .filter(|t| t.is_finite() && *t >= 0.0)
        .collect()
}

/// Merge candidate anchor timestamps, drop near-duplicates (within `min_gap`
/// seconds of another anchor), and cap total count to `max_frames` by keeping
/// an evenly-spaced subset. Returns a sorted Vec.
pub(crate) fn merge_anchors(
    scene: Vec<f64>,
    silence: Vec<f64>,
    duration: f64,
    max_frames: usize,
    min_gap: f64,
) -> Vec<f64> {
    let mut all: Vec<f64> = scene
        .into_iter()
        .chain(silence)
        .filter(|t| t.is_finite() && *t >= 0.0 && (duration <= 0.0 || *t <= duration))
        .collect();
    all.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    // Deduplicate anchors within min_gap of each other, keeping the first.
    let mut deduped: Vec<f64> = Vec::with_capacity(all.len());
    for t in all {
        if deduped.last().is_none_or(|last| (t - *last) > min_gap) {
            deduped.push(t);
        }
    }

    if max_frames == 0 || deduped.len() <= max_frames {
        return deduped;
    }
    // Subsample evenly so we keep the shape of the timeline rather than the
    // densest cluster at the front.
    let step = deduped.len() as f64 / max_frames as f64;
    let mut out = Vec::with_capacity(max_frames);
    for i in 0..max_frames {
        let idx = (i as f64 * step).floor() as usize;
        out.push(deduped[idx.min(deduped.len() - 1)]);
    }
    out.dedup_by(|a, b| (*a - *b).abs() < f64::EPSILON);
    out
}

/// Run scene-change and silence detection, merge + cap, return anchor times.
/// The detection passes run sequentially to keep ffmpeg from thrashing two
/// decodes at once on the same file; both are O(duration).
async fn detect_anchors(
    ffmpeg: &Path,
    video_path: &Path,
    metadata: &VideoMetadata,
    config: &FrameConfig,
    on_tick: Arc<dyn Fn(f64, String) + Send + Sync>,
) -> Result<Vec<f64>, NarratorError> {
    // Emit a more informative label so the user knows what's happening
    // during the ffmpeg detect passes (which can each run for many seconds
    // on a long source — the progress bar would otherwise sit at 2% silently).
    on_tick(
        0.02,
        format!(
            "Detecting scene changes in {:.0}s video",
            metadata.duration_seconds
        ),
    );
    let scene = detect_scene_changes(ffmpeg, video_path, config.scene_threshold).await?;
    on_tick(0.10, format!("Found {} scene changes", scene.len()));

    let silence = match probe_has_audio_stream(video_path).await {
        Ok(true) => {
            on_tick(0.12, "Detecting silence boundaries".to_string());
            let found = detect_silence_boundaries(ffmpeg, video_path)
                .await
                .unwrap_or_default();
            on_tick(0.20, format!("Found {} silence boundaries", found.len()));
            found
        }
        _ => Vec::new(),
    };

    let anchors = merge_anchors(
        scene,
        silence,
        metadata.duration_seconds,
        config.max_frames,
        1.0,
    );
    tracing::info!(
        "anchor sampling: {} anchors over {:.1}s (scene_threshold={:.2})",
        anchors.len(),
        metadata.duration_seconds,
        config.scene_threshold
    );
    Ok(anchors)
}

async fn detect_scene_changes(
    ffmpeg: &Path,
    video_path: &Path,
    threshold: f64,
) -> Result<Vec<f64>, NarratorError> {
    let threshold = threshold.clamp(0.05, 0.95);
    let filter = format!("select='gt(scene,{threshold:.3})',showinfo");
    let output = Command::new(ffmpeg.as_os_str())
        .no_window()
        .args(["-nostats", "-hide_banner", "-i"])
        .arg(video_path.as_os_str())
        .args(["-vf", &filter, "-vsync", "vfr", "-f", "null", "-"])
        .output()
        .await
        .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        // showinfo pipes data on success; non-success here means the detect
        // pass itself failed and we should not pretend we have anchors.
        return Err(NarratorError::FfmpegFailed(
            stderr.lines().rev().take(3).collect::<Vec<_>>().join("\n"),
        ));
    }
    Ok(parse_showinfo_timestamps(&stderr))
}

async fn detect_silence_boundaries(
    ffmpeg: &Path,
    video_path: &Path,
) -> Result<Vec<f64>, NarratorError> {
    let output = Command::new(ffmpeg.as_os_str())
        .no_window()
        .args(["-nostats", "-hide_banner", "-i"])
        .arg(video_path.as_os_str())
        .args(["-af", "silencedetect=n=-30dB:d=0.5", "-f", "null", "-"])
        .output()
        .await
        .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;
    // silencedetect writes detection lines to stderr even on success.
    let stderr = String::from_utf8_lossy(&output.stderr);
    Ok(parse_silence_midpoints(&stderr))
}

/// Coarse-seek window (seconds) subtracted from an anchor timestamp before
/// the `-ss BEFORE -i` keyframe jump. The `-ss AFTER -i` decode then walks
/// forward this much to the exact anchor. Larger = more decode cost, smaller
/// = risk of landing on the wrong side of sparse keyframes (some screen
/// recordings GOP every 10s). 2.0s handles most practical GOPs while keeping
/// the per-anchor decode cheap.
const ANCHOR_COARSE_SEEK_PAD_SECS: f64 = 2.0;

/// Extract one frame per anchor timestamp via independent ffmpeg invocations.
///
/// Uses a two-step seek for frame accuracy:
/// - `-ss <coarse>` BEFORE `-i` jumps to the nearest keyframe at-or-before
///   `anchor - PAD` (fast, decodes nothing).
/// - `-ss <fine>` AFTER `-i` decodes forward to the exact anchor (frame-
///   accurate, costs ~PAD seconds of decode per anchor).
///
/// A single-pass input seek would overshoot backward by up to a full GOP on
/// sparse-keyframe sources (typical screencasts), which defeats the whole
/// feature: an anchor placed at a scene cut would extract the pre-cut frame.
/// The two-step seek is the textbook ffmpeg fix for this.
///
/// Checks `cancel_flag` between anchors so a user pressing "cancel" during a
/// 30-anchor extraction doesn't have to wait through all remaining
/// invocations.
#[allow(clippy::too_many_arguments)]
async fn extract_frames_at_anchors(
    ffmpeg: &Path,
    video_path: &Path,
    anchors: &[f64],
    metadata: &VideoMetadata,
    output_dir: &Path,
    on_frame: Arc<dyn Fn(Frame) + Send + Sync>,
    on_tick: Arc<dyn Fn(f64, String) + Send + Sync>,
    cancel_flag: Option<Arc<AtomicBool>>,
) -> Result<Vec<Frame>, NarratorError> {
    on_tick(
        0.30,
        format!("Extracting {} anchored frames", anchors.len()),
    );

    let mut frames: Vec<Frame> = Vec::with_capacity(anchors.len());
    let total = anchors.len().max(1) as f64;
    for (i, &ts) in anchors.iter().enumerate() {
        check_cancelled(&cancel_flag)?;

        let out_path = output_dir.join(format!("frame_{:04}.jpg", i + 1));
        let coarse = (ts - ANCHOR_COARSE_SEEK_PAD_SECS).max(0.0);
        let fine = ts - coarse;
        let coarse_str = format!("{coarse:.3}");
        let fine_str = format!("{fine:.3}");
        let status = Command::new(ffmpeg.as_os_str())
            .no_window()
            .args(["-nostats", "-hide_banner", "-y", "-ss", &coarse_str, "-i"])
            .arg(video_path.as_os_str())
            .args(["-ss", &fine_str, "-frames:v", "1", "-q:v", "2"])
            .arg(out_path.as_os_str())
            .output()
            .await
            .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;
        if !status.status.success() {
            tracing::warn!(
                "anchor frame at {:.2}s failed, skipping ({})",
                ts,
                String::from_utf8_lossy(&status.stderr)
                    .lines()
                    .last()
                    .unwrap_or("")
            );
            continue;
        }
        let Some((width, height)) = get_image_dimensions(&out_path) else {
            tracing::warn!(
                "skipping anchor frame with unreadable dimensions: {}",
                out_path.display()
            );
            continue;
        };
        let frame = Frame {
            index: frames.len(),
            timestamp_seconds: ts.min(metadata.duration_seconds.max(0.0)),
            path: out_path,
            width,
            height,
        };
        on_frame(frame.clone());
        frames.push(frame);

        let fraction = 0.30 + ((i + 1) as f64 / total) * 0.65;
        on_tick(
            fraction,
            format!("Extracted anchor frame {} of {}", i + 1, anchors.len()),
        );
    }

    on_tick(1.0, format!("Extracted {} frames", frames.len()));
    Ok(frames)
}

/// The historical fixed-interval extraction path. Kept as a fallback for
/// silent / static / very short videos where anchor detection produces too
/// few candidates to be useful.
async fn extract_frames_fixed_interval(
    ffmpeg: &Path,
    video_path: &Path,
    metadata: &VideoMetadata,
    config: &FrameConfig,
    output_dir: &Path,
    on_frame: Arc<dyn Fn(Frame) + Send + Sync>,
    on_tick: Arc<dyn Fn(f64, String) + Send + Sync>,
) -> Result<Vec<Frame>, NarratorError> {
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

    #[test]
    fn resolve_duration_prefers_video_stream_over_longer_audio() {
        // Regression: a previously-narrated Narrator export has audio >> video.
        // Without this fix probe_video returned 231.888 (audio) instead of
        // 104.833 (video), so the AI generated 3:51 of narration for a 1:44
        // video.
        let stream = serde_json::json!({ "duration": "104.833300" });
        let format = serde_json::json!({ "duration": "231.888000" });
        let d = resolve_video_duration(&stream, &format);
        assert!((d - 104.8333).abs() < 1e-4, "got {d}");
    }

    #[test]
    fn resolve_duration_falls_back_to_format_when_stream_missing() {
        // WebM and some MKV files don't expose per-stream duration — use
        // format.duration instead of failing.
        let stream = serde_json::json!({});
        let format = serde_json::json!({ "duration": "60.0" });
        assert!((resolve_video_duration(&stream, &format) - 60.0).abs() < 1e-9);
    }

    #[test]
    fn resolve_duration_ignores_na_and_zero() {
        // ffprobe occasionally emits "N/A" or "0.000000" for unreadable streams.
        // Both should fall through to the next source rather than poisoning
        // the result.
        let stream = serde_json::json!({ "duration": "N/A" });
        let format = serde_json::json!({ "duration": "0.000000" });
        assert_eq!(resolve_video_duration(&stream, &format), 0.0);

        let stream2 = serde_json::json!({ "duration": "0" });
        let format2 = serde_json::json!({ "duration": "42.5" });
        assert!((resolve_video_duration(&stream2, &format2) - 42.5).abs() < 1e-9);
    }

    #[test]
    fn resolve_duration_uses_stream_even_when_format_shorter() {
        // Defensive — stream duration is authoritative for visual content.
        let stream = serde_json::json!({ "duration": "100.0" });
        let format = serde_json::json!({ "duration": "90.0" });
        assert!((resolve_video_duration(&stream, &format) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn showinfo_timestamps_parses_multiple_frames() {
        let stderr = r#"
[Parsed_showinfo_1 @ 0x7f] n:   0 pts:    0 pts_time:0     pos:        0 fmt:yuv420p sar:0/1 s:1920x1080 i:P
[Parsed_showinfo_1 @ 0x7f] n:   1 pts: 60000 pts_time:2.5   pos:   200000 fmt:yuv420p sar:0/1 s:1920x1080 i:P
[Parsed_showinfo_1 @ 0x7f] n:   2 pts:120000 pts_time:12.34 pos:   400000 fmt:yuv420p sar:0/1 s:1920x1080 i:P
"#;
        let ts = parse_showinfo_timestamps(stderr);
        assert_eq!(ts, vec![0.0, 2.5, 12.34]);
    }

    #[test]
    fn showinfo_timestamps_ignores_non_pts_lines() {
        let stderr = "Input #0, mov,mp4,m4a,3gp,3g2,mj2\n  Duration: 00:01:30.00, start: 0.000000\n[something else] pts_time:99.0\n";
        let ts = parse_showinfo_timestamps(stderr);
        // The line has pts_time, so we take it — we intentionally don't try
        // to disambiguate showinfo from other filters' log lines.
        assert_eq!(ts, vec![99.0]);
    }

    #[test]
    fn silence_midpoints_pairs_starts_with_ends() {
        let stderr = r#"
[silencedetect @ 0x7f] silence_start: 1.0
[silencedetect @ 0x7f] silence_end: 2.0 | silence_duration: 1.0
[silencedetect @ 0x7f] silence_start: 10.5
[silencedetect @ 0x7f] silence_end: 11.5 | silence_duration: 1.0
"#;
        let mids = parse_silence_midpoints(stderr);
        assert_eq!(mids.len(), 2);
        assert!((mids[0] - 1.5).abs() < 1e-9);
        assert!((mids[1] - 11.0).abs() < 1e-9);
    }

    #[test]
    fn silence_midpoints_drops_unpaired_trailing_start() {
        // silencedetect occasionally leaves an open silence at EOF without
        // emitting a silence_end line. Treat that as "no midpoint available."
        let stderr = r#"
[silencedetect @ 0x7f] silence_start: 1.0
[silencedetect @ 0x7f] silence_end: 2.0 | silence_duration: 1.0
[silencedetect @ 0x7f] silence_start: 99.0
"#;
        let mids = parse_silence_midpoints(stderr);
        assert_eq!(mids, vec![1.5]);
    }

    #[test]
    fn merge_anchors_deduplicates_within_gap() {
        let scene = vec![1.0, 5.0, 5.4, 10.0];
        let silence = vec![1.1, 7.0];
        let merged = merge_anchors(scene, silence, 30.0, 10, 1.0);
        // 1.0 kept, 1.1 dropped (within 1.0s). 5.0 kept, 5.4 dropped.
        assert_eq!(merged, vec![1.0, 5.0, 7.0, 10.0]);
    }

    #[test]
    fn merge_anchors_caps_to_max_frames_with_even_spacing() {
        let scene: Vec<f64> = (0..100).map(|i| i as f64 * 0.5).collect();
        let merged = merge_anchors(scene, vec![], 60.0, 5, 0.1);
        assert_eq!(merged.len(), 5);
        // Even spacing across the timeline — first near start, last near end.
        assert!(merged[0] < 5.0);
        assert!(merged[4] > 30.0);
    }

    #[test]
    fn merge_anchors_drops_out_of_range_and_nan() {
        let scene = vec![-1.0, 5.0, f64::NAN, 100.0];
        let merged = merge_anchors(scene, vec![], 30.0, 10, 1.0);
        assert_eq!(merged, vec![5.0]);
    }

    #[test]
    fn check_cancelled_passes_when_flag_absent_or_false() {
        assert!(check_cancelled(&None).is_ok());
        let flag = Arc::new(AtomicBool::new(false));
        assert!(check_cancelled(&Some(flag)).is_ok());
    }

    #[test]
    fn check_cancelled_returns_err_when_flag_set() {
        let flag = Arc::new(AtomicBool::new(true));
        let err = check_cancelled(&Some(flag)).unwrap_err();
        assert!(matches!(err, NarratorError::Cancelled));
    }
}
