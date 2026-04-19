//! Audio for the single-pass pipeline.
//!
//! Each clip in a `VideoEditPlan` references a `[start, end]` slice of the
//! source audio at a given playback `speed`. To assemble the timeline's
//! audio in one ffmpeg pass, we build a `filter_complex` graph of
//! `atrim → atempo → asetpts` segments and concat them. The result is a
//! single PCM WAV the video encoder will mux on its second input.
//!
//! atempo only accepts factors in [0.5, 2.0]; outside that range we chain
//! it (e.g. `atempo=2,atempo=2` for 4×). `atempo=1.0` is omitted for clarity.
//!
//! Freeze clips drop audio (no source media playing during a still frame),
//! matching the existing `process_freeze_clip` behaviour.

use std::path::{Path, PathBuf};

use tokio::process::Command;

use crate::error::NarratorError;
use crate::process_utils::CommandNoWindow;
use crate::video_edit::EditClip;
use crate::video_engine;

/// Build the per-clip audio segments and concat them into `out_path` as PCM
/// stereo s16 at 48kHz. Returns `Ok(None)` when the plan has no audible
/// content (all freeze, or all skip_frames).
pub async fn render_timeline_audio(
    source: &Path,
    clips: &[EditClip],
    out_path: &Path,
) -> Result<Option<PathBuf>, NarratorError> {
    let segs: Vec<(usize, &EditClip)> = clips
        .iter()
        .enumerate()
        .filter(|(_, c)| c.clip_type.as_deref() != Some("freeze") && !c.skip_frames)
        .collect();

    if segs.is_empty() {
        return Ok(None);
    }

    let ffmpeg = video_engine::detect_ffmpeg()?;

    // Build per-clip filter chains, e.g.
    //   [0:a]atrim=start=1.0:end=3.0,asetpts=PTS-STARTPTS,atempo=2.0[a0]
    let mut filters: Vec<String> = Vec::with_capacity(segs.len());
    let mut labels: Vec<String> = Vec::with_capacity(segs.len());
    for (idx, clip) in &segs {
        let label = format!("a{idx}");
        let mut chain = format!(
            "[0:a]atrim=start={s}:end={e},asetpts=PTS-STARTPTS",
            s = clip.start_seconds,
            e = clip.end_seconds
        );
        for tempo in atempo_factors(clip.speed) {
            chain.push_str(&format!(",atempo={tempo}"));
        }
        chain.push_str(&format!("[{label}]"));
        filters.push(chain);
        labels.push(label);
    }

    let concat_inputs: String = labels.iter().map(|l| format!("[{l}]")).collect();
    let concat_filter = format!(
        "{concat_inputs}concat=n={n}:v=0:a=1[aout]",
        n = labels.len()
    );

    let filter_complex = format!("{};{}", filters.join(";"), concat_filter);

    let output = Command::new(ffmpeg.as_os_str())
        .no_window()
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(source.as_os_str())
        .args([
            "-filter_complex",
            &filter_complex,
            "-map",
            "[aout]",
            "-c:a",
            "pcm_s16le",
            "-ar",
            "48000",
            "-ac",
            "2",
        ])
        .arg(out_path.as_os_str())
        .output()
        .await
        .map_err(|e| NarratorError::FfmpegFailed(format!("audio render: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // If the source has no audio stream, ffmpeg fails — return None so
        // the encoder runs video-only without this being a hard error.
        if stderr.contains("does not contain any stream")
            || stderr.contains("Stream specifier 'a' in filtergraph")
            || stderr.contains("matches no streams")
        {
            return Ok(None);
        }
        return Err(NarratorError::FfmpegFailed(format!(
            "audio render exited {:?}: {}",
            output.status.code(),
            &stderr[stderr.len().saturating_sub(400)..]
        )));
    }
    Ok(Some(out_path.to_path_buf()))
}

/// Decompose any speed factor into a chain of atempo values each in [0.5, 2.0].
/// Returns an empty vec for speed == 1.0.
fn atempo_factors(speed: f64) -> Vec<f64> {
    if !(speed.is_finite()) || speed <= 0.0 {
        return vec![];
    }
    if (speed - 1.0).abs() < 1e-6 {
        return vec![];
    }
    let mut out = Vec::new();
    let mut remaining = speed;
    while remaining > 2.0 {
        out.push(2.0);
        remaining /= 2.0;
    }
    while remaining < 0.5 {
        out.push(0.5);
        remaining *= 2.0;
    }
    out.push(remaining);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atempo_passthrough() {
        assert_eq!(atempo_factors(1.0), Vec::<f64>::new());
    }

    #[test]
    fn atempo_in_range_is_one_step() {
        assert_eq!(atempo_factors(1.5).len(), 1);
        assert_eq!(atempo_factors(0.7).len(), 1);
    }

    #[test]
    fn atempo_chains_above_two() {
        let f = atempo_factors(4.0);
        assert!(f.len() >= 2);
        let product: f64 = f.iter().product();
        assert!((product - 4.0).abs() < 1e-3);
    }

    #[test]
    fn atempo_chains_below_half() {
        let f = atempo_factors(0.25);
        assert!(f.len() >= 2);
        let product: f64 = f.iter().product();
        assert!((product - 0.25).abs() < 1e-3);
    }
}
