//! Video processing engine using ffmpeg for frame extraction and probing.

use crate::cancel::{check_cancelled, is_cancelled, kill_child_after_cancel, output_with_cancel};
use crate::error::NarratorError;
use crate::ffmpeg_progress::{extract_time_from_ffmpeg_line, parse_ffmpeg_time};
use crate::models::{Frame, FrameConfig, SilenceSpan, VideoMetadata};
use crate::process_utils::CommandNoWindow;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;
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

/// True if the detected `ffmpeg` advertises `name` in its filter table.
///
/// Uncached — callers should wrap this in their own `OnceLock` so a given
/// filter costs one ~50 ms `-filters` invocation per process.
fn ffmpeg_advertises_filter(name: &str) -> bool {
    let Ok(ffmpeg) = detect_ffmpeg() else {
        return false;
    };
    let output = std::process::Command::new(ffmpeg.as_os_str())
        .no_window()
        .args(["-hide_banner", "-filters"])
        .output();
    match output {
        Ok(out) if out.status.success() => {
            // Filter table has one row per filter; match at a line boundary
            // so we don't false-positive on the word appearing in a
            // description (e.g. "Render text subtitles...").
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout.lines().any(|line| {
                // Filter rows look like: " .. subtitles  V->V   ..."
                line.split_whitespace().nth(1) == Some(name)
            })
        }
        _ => false,
    }
}

/// True if the detected `ffmpeg` has the `subtitles` video filter (libass).
///
/// Burning subtitles into a video needs libass-backed `-vf subtitles=...`.
/// Homebrew's default `ffmpeg` formula ships without libass and silently
/// produces the very confusing "Error parsing a filter description" error.
/// Result is cached so we pay ~50 ms once per process.
pub fn ffmpeg_has_subtitles_filter() -> bool {
    use std::sync::OnceLock;
    static CACHED: OnceLock<bool> = OnceLock::new();
    *CACHED.get_or_init(|| ffmpeg_advertises_filter("subtitles"))
}

/// True if the detected `ffmpeg` has `zscale` (libzimg).
///
/// The HDR tone-mapping chain is built on `zscale`, which only exists in
/// builds linked against libzimg. Our bundled sidecars have it; a system
/// ffmpeg a developer happens to have on PATH may not. Checked rather than
/// assumed, because an unavailable filter fails the *entire* encode — a much
/// worse outcome than the washed-out colours we're trying to fix.
pub fn ffmpeg_has_zscale_filter() -> bool {
    use std::sync::OnceLock;
    static CACHED: OnceLock<bool> = OnceLock::new();
    *CACHED.get_or_init(|| ffmpeg_advertises_filter("zscale"))
}

// ── HDR → SDR tone mapping ───────────────────────────────────────────────────
//
// iPhone captures and modern screen recorders increasingly write HDR: PQ
// (`smpte2084`) or HLG (`arib-std-b67`) transfer curves. Re-encoding those to
// 8-bit `yuv420p` without tone mapping does not fail — it silently produces
// washed-out, grey, low-contrast video, because the HDR curve is reinterpreted
// as if it were Rec.709.
//
// This bites twice in Narrator. The obvious damage is the exported video. The
// less obvious damage is that the JPEG frames we send to the vision model come
// out of the same pipeline, so the model reasons about washed-out screenshots
// and describes them less accurately.

/// Transfer characteristics that require tone mapping before an SDR encode.
///
/// `bt709`, `smpte170m`, `iec61966-2-1` (sRGB) and friends are already SDR and
/// must be left alone — tone mapping them would crush contrast.
pub fn is_hdr_transfer(transfer: &str) -> bool {
    matches!(transfer.trim(), "smpte2084" | "arib-std-b67")
}

/// Tone-map HDR to Rec.709 SDR.
///
/// Deliberately stops before any `format=`/`scale=` step so callers can append
/// their own target (`format=yuv420p` for an encode, `scale=…,format=rgba` for
/// the compositor's raw pipe) without a redundant conversion.
///
/// `npl=100` treats the source as 100-nit-referenced, and `hable` is the
/// tone-mapping curve that preserves highlight detail without the flat look
/// `clip` or `linear` give. `desat=0` disables ffmpeg's highlight desaturation,
/// which otherwise greys out bright UI on screen recordings.
pub const HDR_TO_SDR_FILTER: &str = "zscale=t=linear:npl=100,format=gbrpf32le,\
     zscale=p=bt709,tonemap=tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv";

/// Prepend the tone-map chain to an existing filter string.
///
/// Tone mapping must run *first*: everything downstream (scaling, subtitle
/// burn-in, overlays) should operate on Rec.709 pixels. Returns `filter`
/// untouched when `hdr` is false, and handles an empty `filter`.
pub fn prepend_hdr_tonemap(filter: &str, hdr: bool) -> String {
    if !hdr {
        return filter.to_string();
    }
    if filter.trim().is_empty() {
        return HDR_TO_SDR_FILTER.to_string();
    }
    format!("{HDR_TO_SDR_FILTER},{filter}")
}

/// Probe the colour transfer characteristic of the first video stream.
///
/// Returns `Ok(None)` when ffprobe parsed the file but the stream declares no
/// transfer (very common — most SDR files omit it). `Err` only when ffprobe
/// itself failed to run.
pub async fn probe_color_transfer(path: &Path) -> Result<Option<String>, NarratorError> {
    probe_video_stream_field(path, "color_transfer").await
}

/// Read one field of the first video stream via ffprobe.
async fn probe_video_stream_field(
    path: &Path,
    field: &str,
) -> Result<Option<String>, NarratorError> {
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

    Ok(json["streams"][0][field].as_str().map(|s| s.to_string()))
}

/// Best-effort: should this source be tone-mapped before an SDR encode?
///
/// Never returns an error. A probe failure, a missing `color_transfer`, or an
/// ffmpeg without `zscale` all resolve to `false` — i.e. today's behaviour.
/// Producing a slightly washed-out file is recoverable; failing the export
/// because a filter is unavailable is not.
pub async fn needs_hdr_tonemap(path: &Path) -> bool {
    let transfer = match probe_color_transfer(path).await {
        Ok(Some(t)) => t,
        Ok(None) => return false,
        Err(e) => {
            tracing::debug!("colour transfer probe failed, assuming SDR: {e}");
            return false;
        }
    };

    if !is_hdr_transfer(&transfer) {
        return false;
    }

    if !ffmpeg_has_zscale_filter() {
        tracing::warn!(
            "source is HDR ({transfer}) but this ffmpeg has no zscale filter — \
             skipping tone mapping; colours may look washed out"
        );
        return false;
    }

    tracing::info!("source is HDR ({transfer}) — tone mapping to Rec.709");
    true
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
    probe_video_stream_field(path, "pix_fmt").await
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
/// `anchor_override`, when supplied with at least `MIN_ANCHORS` entries, bypasses
/// scene detection entirely — used by the model-driven selection path, where the
/// timestamps were already chosen from a survey and re-deriving them would waste
/// a full decode pass. Silence detection still runs, because the narration
/// timeline needs that map however the frames were picked.
#[allow(clippy::too_many_arguments)]
pub async fn extract_frames_with_anchors(
    video_path: &Path,
    config: &FrameConfig,
    output_dir: &Path,
    anchor_override: Option<Vec<f64>>,
    on_frame: impl Fn(Frame) + Send + Sync + 'static,
    on_tick: impl Fn(f64, String) + Send + Sync + 'static,
    cancel_flag: Option<Arc<AtomicBool>>,
) -> Result<FrameExtraction, NarratorError> {
    let ffmpeg = detect_ffmpeg()?;

    // Ensure output dir exists
    tokio::fs::create_dir_all(output_dir)
        .await
        .map_err(|e| NarratorError::FrameExtractionError(e.to_string()))?;

    let metadata = probe_video(video_path).await?;

    // Probed once here rather than per-frame: an HDR source decoded straight to
    // JPEG yields washed-out frames, and the vision model then reasons about
    // washed-out screenshots.
    let hdr = needs_hdr_tonemap(video_path).await;

    let on_frame: Arc<dyn Fn(Frame) + Send + Sync> = Arc::new(on_frame);
    let on_tick: Arc<dyn Fn(f64, String) + Send + Sync> = Arc::new(on_tick);

    // Attempt anchor-based sampling first; fall back to fixed-interval if it
    // didn't yield enough anchors or if any detection step errored. We need
    // at least `MIN_ANCHORS` frames for the fallback threshold to feel
    // meaningful — fewer than that and the LLM can't tell scene structure
    // from a handful of samples.
    const MIN_ANCHORS: usize = 3;

    // Model-chosen anchors skip scene detection entirely — re-deriving anchors
    // we already have would waste a full decode pass. Silence detection still
    // runs, because the narration timeline needs the map however frames were
    // picked.
    if let Some(anchors) = anchor_override.filter(|a| a.len() >= MIN_ANCHORS) {
        on_tick(
            0.05,
            format!("Extracting {} model-selected frames", anchors.len()),
        );
        let silence_spans = match probe_has_audio_stream(video_path).await {
            Ok(true) => detect_silence_spans(&ffmpeg, video_path, &cancel_flag)
                .await
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        let frames = extract_frames_at_anchors(
            &ffmpeg,
            video_path,
            &anchors,
            &metadata,
            output_dir,
            on_frame,
            on_tick,
            cancel_flag,
            hdr,
        )
        .await?;
        return Ok(FrameExtraction {
            frames,
            silence_spans,
        });
    }

    // Silence spans survive the fixed-interval fallback: they describe the
    // audio, not the sampling strategy, and the narration timeline needs them
    // either way.
    let mut silence_spans: Vec<SilenceSpan> = Vec::new();
    match detect_anchors(
        &ffmpeg,
        video_path,
        &metadata,
        config,
        on_tick.clone(),
        &cancel_flag,
    )
    .await
    {
        Ok((anchors, spans)) => {
            silence_spans = spans;
            if anchors.len() >= MIN_ANCHORS {
                let frames = extract_frames_at_anchors(
                    &ffmpeg,
                    video_path,
                    &anchors,
                    &metadata,
                    output_dir,
                    on_frame,
                    on_tick,
                    cancel_flag,
                    hdr,
                )
                .await?;
                return Ok(FrameExtraction {
                    frames,
                    silence_spans,
                });
            }
            tracing::info!(
                "anchor-based sampling found only {} frames (< {}), falling back to fixed interval",
                anchors.len(),
                MIN_ANCHORS
            );
        }
        Err(_) => {
            tracing::warn!("anchor detection failed, falling back to fixed interval");
        }
    }

    let frames = extract_frames_fixed_interval(
        &ffmpeg,
        video_path,
        &metadata,
        config,
        output_dir,
        ExtractionControl {
            on_frame,
            on_tick,
            cancel_flag,
        },
        hdr,
    )
    .await?;
    Ok(FrameExtraction {
        frames,
        silence_spans,
    })
}

/// Extract a dense, low-resolution survey of the whole video in ONE ffmpeg pass.
///
/// Deliberately unlike [`extract_frames_at_anchors`], which spawns a process per
/// anchor and pays a two-step seek each time for frame accuracy. A survey needs
/// coverage, not precision: a single `fps=` filter pass over one decode is orders
/// of magnitude cheaper, and being a few frames off does not matter when the
/// output is "which timestamps deserve a closer look".
///
/// Frames are scaled to `width` and written as `survey_%04d.jpg`.
pub async fn extract_survey_frames(
    video_path: &Path,
    output_dir: &Path,
    target_count: usize,
    width: u32,
    cancel: Option<Arc<AtomicBool>>,
) -> Result<Vec<Frame>, NarratorError> {
    let ffmpeg = detect_ffmpeg()?;
    tokio::fs::create_dir_all(output_dir)
        .await
        .map_err(|e| NarratorError::FrameExtractionError(e.to_string()))?;

    let metadata = probe_video(video_path).await?;
    let (fps, _) = crate::frame_selection::plan_survey(metadata.duration_seconds, target_count);
    let hdr = needs_hdr_tonemap(video_path).await;

    // Tone map first (if needed), then select, then scale — same ordering rule as
    // the full-resolution paths.
    let filter = prepend_hdr_tonemap(&format!("fps={fps:.6},scale={width}:-2"), hdr);
    let pattern = output_dir.join("survey_%04d.jpg");

    let mut cmd = Command::new(ffmpeg.as_os_str());
    cmd.no_window()
        .args(["-nostats", "-hide_banner", "-y", "-i"])
        .arg(video_path.as_os_str())
        .args(["-vf", &filter, "-q:v", "4"])
        .arg(pattern.as_os_str());
    let output = output_with_cancel(&mut cmd, &cancel).await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(NarratorError::FfmpegFailed(
            stderr.lines().rev().take(3).collect::<Vec<_>>().join("\n"),
        ));
    }

    // Map each emitted file back to the timestamp it represents. ffmpeg numbers
    // them 1..N in order, and `fps=` spaces them evenly at 1/fps.
    let interval = if fps > 0.0 { 1.0 / fps } else { 1.0 };
    let mut frames = Vec::new();
    for i in 0.. {
        let path = output_dir.join(format!("survey_{:04}.jpg", i + 1));
        if !path.is_file() {
            break;
        }
        let Some((w, h)) = get_image_dimensions(&path) else {
            continue;
        };
        frames.push(Frame {
            index: i,
            timestamp_seconds: (i as f64 * interval).min(metadata.duration_seconds.max(0.0)),
            path,
            width: w,
            height: h,
        });
    }

    tracing::info!(
        "survey pass: {} frames at {width}px over {:.1}s (one decode)",
        frames.len(),
        metadata.duration_seconds
    );
    Ok(frames)
}

/// What one extraction pass learned about the source.
///
/// The silence map is a by-product of anchor detection that used to be thrown
/// away. Returning it means the narration timeline can avoid placing speech
/// over existing audio without paying for a second decode pass.
#[derive(Debug, Clone, Default)]
pub struct FrameExtraction {
    pub frames: Vec<Frame>,
    pub silence_spans: Vec<SilenceSpan>,
}

struct ExtractionControl {
    on_frame: Arc<dyn Fn(Frame) + Send + Sync>,
    on_tick: Arc<dyn Fn(f64, String) + Send + Sync>,
    cancel_flag: Option<Arc<AtomicBool>>,
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

/// Parse `silencedetect` stderr into full silence spans.
///
/// Spans, not midpoints: the narration timeline needs to know how *wide* each
/// quiet stretch is, so it can place a segment edge inside one instead of
/// starting a sentence over existing speech. Anchor selection collapses these
/// to midpoints separately.
pub(crate) fn parse_silence_spans(stderr: &str) -> Vec<SilenceSpan> {
    let (starts, ends) = parse_silence_events(stderr);
    // Pair positionally. silencedetect always logs a start before its matching
    // end, but a run can finish with an unterminated silence (ending at EOF)
    // that has no `silence_end` line — ignore those unpaired trailing starts.
    let pairs = starts.len().min(ends.len());
    (0..pairs)
        .map(|i| SilenceSpan {
            start: starts[i],
            end: ends[i],
        })
        .filter(|s| s.start.is_finite() && s.end.is_finite() && s.end > s.start)
        .collect()
}

/// Extract the raw `silence_start:` / `silence_end:` timestamp lists.
fn parse_silence_events(stderr: &str) -> (Vec<f64>, Vec<f64>) {
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
    (starts, ends)
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
    cancel: &Option<Arc<AtomicBool>>,
) -> Result<(Vec<f64>, Vec<SilenceSpan>), NarratorError> {
    check_cancelled(cancel)?;
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
    let scene = detect_scene_changes(ffmpeg, video_path, config.scene_threshold, cancel).await?;
    on_tick(0.10, format!("Found {} scene changes", scene.len()));
    check_cancelled(cancel)?;

    let silence_spans = match probe_has_audio_stream(video_path).await {
        Ok(true) => {
            on_tick(0.12, "Detecting silence boundaries".to_string());
            let found = detect_silence_spans(ffmpeg, video_path, cancel)
                .await
                .unwrap_or_default();
            on_tick(0.20, format!("Found {} silence spans", found.len()));
            found
        }
        _ => Vec::new(),
    };
    // `detect_silence_spans` swallows its own errors via unwrap_or_default,
    // so re-check here to make sure a cancel during that pass propagates.
    check_cancelled(cancel)?;

    // Anchors keep the historical ≥0.5 s threshold even though detection now
    // runs wider (see `SILENCE_MIN_DURATION`): a 0.2 s inter-word gap is a
    // useful narration window but a poor frame anchor, and loosening it here
    // would change which frames the model sees.
    let anchor_midpoints: Vec<f64> = silence_spans
        .iter()
        .filter(|s| s.duration() >= ANCHOR_MIN_SILENCE)
        .map(SilenceSpan::midpoint)
        .collect();

    let anchors = merge_anchors(
        scene,
        anchor_midpoints,
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
    Ok((anchors, silence_spans))
}

async fn detect_scene_changes(
    ffmpeg: &Path,
    video_path: &Path,
    threshold: f64,
    cancel: &Option<Arc<AtomicBool>>,
) -> Result<Vec<f64>, NarratorError> {
    let threshold = threshold.clamp(0.05, 0.95);
    let filter = format!("select='gt(scene,{threshold:.3})',showinfo");
    let mut cmd = Command::new(ffmpeg.as_os_str());
    cmd.no_window()
        .args(["-nostats", "-hide_banner", "-i"])
        .arg(video_path.as_os_str())
        .args(["-vf", &filter, "-vsync", "vfr", "-f", "null", "-"]);
    let output = output_with_cancel(&mut cmd, cancel).await?;
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

/// Shortest silence `silencedetect` will report, in seconds.
///
/// Was 0.5, which is far above the inter-phrase pauses narration can actually
/// use. 0.15 is the floor of the usable band (see `ai_client`'s snap ladder):
/// anything shorter is mid-phrase and unsafe to start speaking over.
const SILENCE_MIN_DURATION: f64 = 0.15;

/// Minimum silence duration that still makes a good *frame anchor*.
///
/// Deliberately stricter than `SILENCE_MIN_DURATION`: a 0.2 s inter-word gap is
/// a fine narration window but a poor place to sample a frame, and this
/// preserves the anchor behaviour that predates span detection.
const ANCHOR_MIN_SILENCE: f64 = 0.5;

// Detection must stay at least as loose as the narrowest gap the snap pass will
// use, or the usable 150-400 ms band is invisible to it. Checked at compile time
// so the two constants cannot drift apart in a later edit.
const _: () = assert!(SILENCE_MIN_DURATION <= crate::ai_client::MIN_SNAP_GAP);
const _: () = assert!(ANCHOR_MIN_SILENCE > SILENCE_MIN_DURATION);

/// Noise floor `silencedetect` treats as silence, in dBFS.
///
/// -30 dB is deliberately conservative for the narration pass: screen
/// recordings carry fan and room noise well above a true digital zero, and
/// under-detecting a gap only costs a narration window, while over-detecting
/// one would place speech on top of real audio.
const SILENCE_NOISE_DB: f64 = -30.0;

async fn detect_silence_spans(
    ffmpeg: &Path,
    video_path: &Path,
    cancel: &Option<Arc<AtomicBool>>,
) -> Result<Vec<SilenceSpan>, NarratorError> {
    detect_silence_spans_tuned(
        ffmpeg,
        video_path,
        SILENCE_NOISE_DB,
        SILENCE_MIN_DURATION,
        cancel,
    )
    .await
}

/// `silencedetect` with a caller-chosen noise floor and minimum duration.
///
/// The narration pass wants every usable inter-phrase gap (0.15 s at -30 dB);
/// dead-air trimming wants only stretches long enough that cutting them reads
/// as tightening rather than clipping, and lets the user move both knobs. Both
/// callers share this one ffmpeg pass so the parsing stays in one place.
pub async fn detect_silence_spans_tuned(
    ffmpeg: &Path,
    video_path: &Path,
    noise_db: f64,
    min_duration: f64,
    cancel: &Option<Arc<AtomicBool>>,
) -> Result<Vec<SilenceSpan>, NarratorError> {
    // ffmpeg wants the threshold as a signed dB value (e.g. `n=-30dB`), and a
    // positive duration; clamp rather than trust the frontend.
    let noise_db = noise_db.clamp(-90.0, 0.0);
    let min_duration = min_duration.max(0.01);
    let filter = format!("silencedetect=n={noise_db}dB:d={min_duration}");
    let mut cmd = Command::new(ffmpeg.as_os_str());
    cmd.no_window()
        .args(["-nostats", "-hide_banner", "-i"])
        .arg(video_path.as_os_str())
        .args(["-af", &filter, "-f", "null", "-"]);
    let output = output_with_cancel(&mut cmd, cancel).await?;
    // silencedetect writes detection lines to stderr even on success.
    let stderr = String::from_utf8_lossy(&output.stderr);
    Ok(parse_silence_spans(&stderr))
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
    hdr: bool,
) -> Result<Vec<Frame>, NarratorError> {
    // Only an HDR source gets a filter chain here; SDR keeps the original
    // filter-free invocation so the common path is untouched.
    let tonemap = prepend_hdr_tonemap("", hdr);
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
        let mut cmd = Command::new(ffmpeg.as_os_str());
        cmd.no_window()
            .args(["-nostats", "-hide_banner", "-y", "-ss", &coarse_str, "-i"])
            .arg(video_path.as_os_str())
            .args(["-ss", &fine_str, "-frames:v", "1", "-q:v", "2"]);
        if !tonemap.is_empty() {
            cmd.args(["-vf", &tonemap]);
        }
        let status = cmd
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
    control: ExtractionControl,
    hdr: bool,
) -> Result<Vec<Frame>, NarratorError> {
    let ExtractionControl {
        on_frame,
        on_tick,
        cancel_flag,
    } = control;

    check_cancelled(&cancel_flag)?;

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
    // Tone mapping runs before the frame selector so the JPEGs are Rec.709.
    let vf_filter = prepend_hdr_tonemap(&format!("fps=1/{interval}"), hdr);

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
        loop {
            tokio::select! {
                line = lines.next_line() => {
                    match line {
                        Ok(Some(line)) => {
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
                        Ok(None) => break,
                        Err(e) => return Err(NarratorError::FfmpegFailed(e.to_string())),
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    if is_cancelled(&cancel_flag) {
                        kill_child_after_cancel(&mut child).await;
                        return Err(NarratorError::Cancelled);
                    }
                }
            }
        }
    }

    if is_cancelled(&cancel_flag) {
        kill_child_after_cancel(&mut child).await;
        return Err(NarratorError::Cancelled);
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
    check_cancelled(&cancel_flag)?;

    // Collect extracted frames — directory scan, image dimension reads, and blake3
    // hashing are CPU/IO-intensive, so run on the blocking thread pool.
    let output_dir_owned = output_dir.to_path_buf();
    let max_frames = config.max_frames;
    let skip_dedup = config.skip_dedup;
    let duration = metadata.duration_seconds;
    let tick_for_blocking = on_tick.clone();
    let cancel_for_blocking = cancel_flag.clone();
    let frames = tokio::task::spawn_blocking(move || {
        check_cancelled(&cancel_for_blocking)?;
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
            check_cancelled(&cancel_for_blocking)?;
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
            check_cancelled(&cancel_for_blocking)?;
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
    use std::sync::atomic::Ordering;

    fn test_metadata() -> VideoMetadata {
        VideoMetadata {
            path: "test.mp4".to_string(),
            duration_seconds: 10.0,
            width: 640,
            height: 360,
            codec: "h264".to_string(),
            fps: 30.0,
            file_size: 1024,
        }
    }

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

    // ── HDR tone mapping ────────────────────────────────────────────────────

    #[test]
    fn hdr_transfer_detects_pq_and_hlg() {
        // The two curves that actually mean HDR in practice.
        assert!(is_hdr_transfer("smpte2084"), "PQ / HDR10");
        assert!(is_hdr_transfer("arib-std-b67"), "HLG");
        // ffprobe values sometimes carry surrounding whitespace.
        assert!(is_hdr_transfer("  smpte2084  "));
    }

    #[test]
    fn hdr_transfer_leaves_sdr_alone() {
        // Tone mapping an SDR source crushes its contrast, so a false positive
        // here is a visible regression on every ordinary screen recording.
        for sdr in [
            "bt709",
            "smpte170m",
            "bt470bg",
            "iec61966-2-1",
            "srgb",
            "linear",
            "unknown",
            "",
        ] {
            assert!(!is_hdr_transfer(sdr), "{sdr} must not be treated as HDR");
        }
    }

    #[test]
    fn tonemap_is_a_no_op_for_sdr_sources() {
        assert_eq!(prepend_hdr_tonemap("fps=1/5", false), "fps=1/5");
        assert_eq!(prepend_hdr_tonemap("", false), "");
    }

    #[test]
    fn tonemap_runs_before_the_existing_filter() {
        let out = prepend_hdr_tonemap("fps=1/5", true);
        assert!(out.ends_with(",fps=1/5"), "existing filter must stay last");
        assert!(out.starts_with("zscale=t=linear"), "tone map must be first");
        let tonemap_at = out.find("tonemap=").expect("tonemap present");
        let fps_at = out.find("fps=1/5").expect("fps present");
        assert!(tonemap_at < fps_at);
    }

    #[test]
    fn tonemap_alone_when_there_was_no_filter() {
        // The anchor path has no `-vf` normally; an HDR source gives it one.
        let out = prepend_hdr_tonemap("", true);
        assert_eq!(out, HDR_TO_SDR_FILTER);
        assert!(!out.ends_with(','), "no dangling separator");
        assert!(!out.starts_with(','));
    }

    #[test]
    fn tonemap_preserves_subtitle_burn_as_the_last_step() {
        // Ordering rule: tone map first, subtitle burn last. libass renders
        // Rec.709 white, so captions must not be pushed through the HDR curve.
        let subtitle = "subtitles=/tmp/x.srt:force_style='FontSize=18'";
        let out = prepend_hdr_tonemap(subtitle, true);
        assert!(out.ends_with(subtitle));
        assert!(out.find("tonemap=").unwrap() < out.find("subtitles=").unwrap());
    }

    #[test]
    fn tonemap_chain_ends_ready_for_a_caller_supplied_format() {
        // The chain must not pin an output pix_fmt: callers append their own
        // (`format=rgba` for the compositor, `yuv420p` for an encode).
        assert!(
            !HDR_TO_SDR_FILTER.ends_with("format=yuv420p"),
            "chain must stay format-agnostic"
        );
        // Composing an rgba target must not leave two conflicting formats last.
        let rgba = prepend_hdr_tonemap("scale=100:100:flags=lanczos,format=rgba", true);
        assert!(rgba.ends_with("format=rgba"));
    }

    #[test]
    fn tonemap_chain_is_a_single_valid_filter_list() {
        // A stray empty element (",,") makes ffmpeg reject the whole graph.
        for part in HDR_TO_SDR_FILTER.split(',') {
            assert!(!part.trim().is_empty(), "empty filter in chain");
        }
        assert!(HDR_TO_SDR_FILTER.contains("zscale=t=linear:npl=100"));
        assert!(HDR_TO_SDR_FILTER.contains("tonemap=tonemap=hable"));
        // Must land back in Rec.709, otherwise the encode is still mislabelled.
        assert!(HDR_TO_SDR_FILTER.contains("zscale=t=bt709:m=bt709:r=tv"));
    }

    #[tokio::test]
    async fn needs_hdr_tonemap_is_false_for_an_unreadable_path() {
        // Best-effort contract: a probe failure degrades to "assume SDR"
        // rather than failing the export.
        assert!(!needs_hdr_tonemap(Path::new("/nonexistent/nope.mp4")).await);
    }

    #[tokio::test]
    async fn fixed_interval_respects_pre_cancelled_flag() {
        let dir = tempfile::tempdir().unwrap();
        let flag = Arc::new(AtomicBool::new(true));
        let err = extract_frames_fixed_interval(
            Path::new("ffmpeg-not-needed"),
            Path::new("video.mp4"),
            &test_metadata(),
            &FrameConfig::default(),
            dir.path(),
            ExtractionControl {
                on_frame: Arc::new(|_| {}),
                on_tick: Arc::new(|_, _| {}),
                cancel_flag: Some(flag),
            },
            false,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, NarratorError::Cancelled));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fixed_interval_cancels_quiet_running_child() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let fake_ffmpeg = dir.path().join("fake-ffmpeg");
        std::fs::write(&fake_ffmpeg, "#!/bin/sh\nsleep 30\n").unwrap();
        let mut perms = std::fs::metadata(&fake_ffmpeg).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake_ffmpeg, perms).unwrap();

        let flag = Arc::new(AtomicBool::new(false));
        let flag_for_task = flag.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            flag_for_task.store(true, Ordering::SeqCst);
        });

        let err = extract_frames_fixed_interval(
            &fake_ffmpeg,
            Path::new("video.mp4"),
            &test_metadata(),
            &FrameConfig::default(),
            dir.path(),
            ExtractionControl {
                on_frame: Arc::new(|_| {}),
                on_tick: Arc::new(|_, _| {}),
                cancel_flag: Some(flag),
            },
            false,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, NarratorError::Cancelled));
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
    fn silence_spans_pair_starts_with_ends() {
        let stderr = r#"
[silencedetect @ 0x7f] silence_start: 1.0
[silencedetect @ 0x7f] silence_end: 2.0 | silence_duration: 1.0
[silencedetect @ 0x7f] silence_start: 10.5
[silencedetect @ 0x7f] silence_end: 11.5 | silence_duration: 1.0
"#;
        let spans = parse_silence_spans(stderr);
        assert_eq!(spans.len(), 2);
        assert_eq!(
            spans[0],
            SilenceSpan {
                start: 1.0,
                end: 2.0
            }
        );
        assert_eq!(
            spans[1],
            SilenceSpan {
                start: 10.5,
                end: 11.5
            }
        );
        // Midpoints still derivable — anchor selection depends on them.
        assert!((spans[0].midpoint() - 1.5).abs() < 1e-9);
        assert!((spans[1].midpoint() - 11.0).abs() < 1e-9);
        assert!((spans[0].duration() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn silence_spans_drop_unpaired_trailing_start() {
        // silencedetect occasionally leaves an open silence at EOF without
        // emitting a silence_end line. Treat that as "no span available."
        let stderr = r#"
[silencedetect @ 0x7f] silence_start: 1.0
[silencedetect @ 0x7f] silence_end: 2.0 | silence_duration: 1.0
[silencedetect @ 0x7f] silence_start: 99.0
"#;
        let spans = parse_silence_spans(stderr);
        assert_eq!(
            spans,
            vec![SilenceSpan {
                start: 1.0,
                end: 2.0
            }]
        );
    }

    #[test]
    fn silence_spans_reject_zero_and_inverted_ranges() {
        // A degenerate span would divide by zero in the snap ladder.
        let stderr = r#"
[silencedetect @ 0x7f] silence_start: 5.0
[silencedetect @ 0x7f] silence_end: 5.0 | silence_duration: 0.0
"#;
        assert!(parse_silence_spans(stderr).is_empty());
    }

    #[test]
    fn silence_spans_are_empty_for_output_with_no_detections() {
        // A fully-loud source produces no silencedetect lines at all.
        assert!(parse_silence_spans("Input #0, mov\n  Duration: 00:01:00.00\n").is_empty());
        assert!(parse_silence_spans("").is_empty());
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
