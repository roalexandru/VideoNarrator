//! Video editing operations: trim, speed, frame dropping, and concatenation.

use crate::error::NarratorError;
use crate::video_engine;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
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
    pub fps_override: Option<u32>,
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

    // If single clip with no modifications, just copy
    if total == 1 && plan.clips[0].speed == 1.0 && plan.clips[0].fps_override.is_none() {
        let clip = &plan.clips[0];
        let output = Command::new(ffmpeg.as_os_str())
            .args([
                "-y",
                "-i",
                input_path,
                "-ss",
                &format!("{:.3}", clip.start_seconds),
                "-to",
                &format!("{:.3}", clip.end_seconds),
                "-c",
                "copy",
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
            vfilters.push(format!("fps={}", fps));
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
            let mut atempo_chain = Vec::new();
            let mut remaining = clip.speed;
            // Chain atempo filters for speeds outside 0.5-2.0
            while remaining > 2.0 {
                atempo_chain.push("atempo=2.0".to_string());
                remaining /= 2.0;
            }
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
            .map(|p| format!("file '{}'", p.to_string_lossy().replace('\'', "'\\''")))
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
            let output2 = Command::new(ffmpeg.as_os_str())
                .args(["-y", "-f", "concat", "-safe", "0", "-i"])
                .arg(concat_list.as_os_str())
                .args(["-c:v", "libx264", "-c:a", "aac", output_path])
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
) -> Result<String, NarratorError> {
    let ffmpeg = video_engine::detect_ffmpeg()?;

    let output = if replace_audio {
        // Replace original audio entirely with narration
        Command::new(ffmpeg.as_os_str())
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
                "-shortest",
                output_path,
            ])
            .output()
            .await
    } else {
        // Mix original + narration audio
        Command::new(ffmpeg.as_os_str())
            .args([
                "-y",
                "-i",
                video_path,
                "-i",
                audio_path,
                "-filter_complex",
                "[0:a][1:a]amix=inputs=2:duration=first:normalize=0[a]",
                "-map",
                "0:v",
                "-map",
                "[a]",
                "-c:v",
                "copy",
                "-c:a",
                "aac",
                output_path,
            ])
            .output()
            .await
    };

    let output = output.map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(NarratorError::FfmpegFailed(format!(
            "Audio merge failed: {}",
            &stderr[..stderr.len().min(300)]
        )));
    }

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
            &format!("fps=1/{:.3},scale=120:-1", interval),
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
