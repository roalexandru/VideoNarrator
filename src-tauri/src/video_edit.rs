//! Video editing operations: trim, speed, frame dropping, and concatenation.

use crate::error::NarratorError;
use crate::video_engine;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoEditPlan {
    pub clips: Vec<EditClip>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditClip {
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub speed: f64,
    #[serde(default)]
    pub skip_frames: bool,
    pub fps_override: Option<f64>,
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
    cmd.args(args)
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

pub async fn apply_edits(
    input_path: &str,
    output_path: &str,
    plan: &VideoEditPlan,
    on_progress: impl Fn(f64),
) -> Result<String, NarratorError> {
    let ffmpeg = video_engine::detect_ffmpeg()?;
    let out_dir = Path::new(output_path).parent().unwrap_or(Path::new("/tmp"));
    let total = plan.clips.len();

    if total == 0 {
        return Err(NarratorError::ExportError("No clips to process".into()));
    }

    // If single clip with no modifications, check if it covers the full source
    if total == 1 && plan.clips[0].speed == 1.0 && plan.clips[0].fps_override.is_none() {
        let clip = &plan.clips[0];

        // Probe original duration to check if the clip covers the full video
        let probe = video_engine::probe_video(std::path::Path::new(input_path)).await?;
        let covers_full =
            clip.start_seconds < 0.5 && (clip.end_seconds - probe.duration_seconds).abs() < 0.5;

        if covers_full {
            // No edits — just use the original file directly (symlink or copy)
            if input_path != output_path {
                std::fs::copy(input_path, output_path)?;
            }
            on_progress(100.0);
            return Ok(output_path.to_string());
        }

        // Trimmed single clip — use accurate seek (input seeking + output duration)
        let duration = clip.end_seconds - clip.start_seconds;
        let output = Command::new(ffmpeg.as_os_str())
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

        let clip_path = out_dir.join(format!("_edit_clip_{:03}.mp4", i));
        let mut args: Vec<String> = vec!["-y".into(), "-i".into(), input_path.into()];

        // Trim
        args.extend(["-ss".into(), format!("{:.3}", clip.start_seconds)]);
        args.extend(["-to".into(), format!("{:.3}", clip.end_seconds)]);

        // Build video filter chain
        let mut vfilters = Vec::new();
        let needs_speed = (clip.speed - 1.0).abs() > 0.01;

        if let Some(fps) = clip.fps_override {
            vfilters.push(format!("fps={:.3}", fps));
        }

        if needs_speed {
            if clip.skip_frames {
                // Frame dropping mode: select every Nth frame, adjust timestamps
                // This produces clean jump cuts instead of fast-forward jitter
                // e.g., at 2x speed, keep every 2nd frame; at 3x, every 3rd
                let n = clip.speed.round().max(2.0) as u32;
                vfilters.push(format!("select='not(mod(n\\,{}))'", n));
                vfilters.push("setpts=N/FRAME_RATE/TB".to_string());
            } else {
                // Normal speed mode: play all frames faster
                vfilters.push(format!("setpts={:.4}*PTS", 1.0 / clip.speed));
            }
        }

        if !vfilters.is_empty() {
            args.extend(["-vf".into(), vfilters.join(",")]);
        }

        // Audio handling for speed changes
        if needs_speed && clip.skip_frames {
            // Skip frames mode: drop audio entirely (it would be choppy)
            args.extend(["-an".into()]);
        } else if needs_speed {
            // ffmpeg atempo supports 0.5-100.0 in a single filter since v4.0
            // Chain only if below 0.5
            let mut atempo_chain = Vec::new();
            let mut remaining = clip.speed;
            while remaining < 0.5 {
                atempo_chain.push("atempo=0.5".to_string());
                remaining /= 0.5;
            }
            atempo_chain.push(format!("atempo={:.4}", remaining));
            args.extend(["-af".into(), atempo_chain.join(",")]);
        } else if vfilters.is_empty() {
            // No filters needed, copy codecs
            args.extend(["-c".into(), "copy".into()]);
        }

        args.push(clip_path.to_string_lossy().to_string());

        let output = Command::new(ffmpeg.as_os_str())
            .args(&args)
            .output()
            .await
            .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::error!("Clip {} failed: {}", i, &stderr[..stderr.len().min(300)]);
            return Err(NarratorError::FfmpegFailed(format!(
                "Clip {i} failed: {}",
                &stderr[..stderr.len().min(200)]
            )));
        }

        clip_files.push(clip_path);
    }

    on_progress(85.0);

    // Concat all clips
    if clip_files.len() == 1 {
        std::fs::rename(&clip_files[0], output_path)?;
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
        std::fs::write(&concat_list, &list_content)?;

        let output = Command::new(ffmpeg.as_os_str())
            .args(["-y", "-f", "concat", "-safe", "0", "-i"])
            .arg(concat_list.as_os_str())
            .args(["-c", "copy", output_path])
            .output()
            .await
            .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;

        if !output.status.success() {
            // Fallback: re-encode concat
            tracing::warn!("Stream-copy concat failed, falling back to re-encode");
            let output2 = Command::new(ffmpeg.as_os_str())
                .args(["-y", "-f", "concat", "-safe", "0", "-i"])
                .arg(concat_list.as_os_str())
                .args([
                    "-c:v",
                    "libx264",
                    "-preset",
                    "medium",
                    "-crf",
                    "18",
                    "-c:a",
                    "aac",
                    "-b:a",
                    "192k",
                    output_path,
                ])
                .output()
                .await
                .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;

            if !output2.status.success() {
                let stderr = String::from_utf8_lossy(&output2.stderr);
                return Err(NarratorError::FfmpegFailed(format!(
                    "Concat failed: {}",
                    &stderr[..stderr.len().min(300)]
                )));
            }
        }

        let _ = std::fs::remove_file(&concat_list);
    }

    // Cleanup temp clips
    for p in &clip_files {
        let _ = std::fs::remove_file(p);
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
                &stderr[..stderr.len().min(300)]
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
                &stderr[..stderr.len().min(300)]
            )));
        }
    }

    on_progress(100.0);
    Ok(output_path.to_string())
}

pub async fn burn_subtitles(
    video_path: &str,
    srt_path: &str,
    output_path: &str,
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
    std::fs::copy(srt_path, &temp_srt)?;

    // Try subtitles filter first (requires libass), fall back to SRT input method
    let srt_path_str = temp_srt
        .to_string_lossy()
        .replace('\\', "/")
        .replace(':', "\\:");
    let subtitle_filter = format!(
        "subtitles='{}':force_style='FontSize=22,PrimaryColour=&H00FFFFFF,OutlineColour=&H00000000,Outline=2,BackColour=&H80000000,Shadow=1,MarginV=30'",
        srt_path_str
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
            let _ = std::fs::remove_file(&temp_srt);
            return Err(NarratorError::FfmpegFailed(format!(
                "Subtitle burn failed: {}",
                &stderr[..stderr.len().min(400)]
            )));
        }
    }

    let _ = std::fs::remove_file(&temp_srt);
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

    std::fs::create_dir_all(output_dir)?;

    let output = Command::new(ffmpeg.as_os_str())
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

    let mut paths: Vec<String> = std::fs::read_dir(output_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "jpg"))
        .map(|e| e.path().to_string_lossy().to_string())
        .collect();
    paths.sort();
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
