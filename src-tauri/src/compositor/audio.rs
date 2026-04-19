//! Audio for the single-pass pipeline + the Rust narration mixer.
//!
//! Phase 5 moves the narration / original-audio mix off ffmpeg `amix` to
//! pure-Rust sample math via `hound`. The benefits:
//!
//! - Per-sample gain ramps via the `Keyframe` machinery — `amix` couldn't
//!   express anything beyond a constant `weights=` per stream.
//! - Energy-based auto-ducking is a few lines of Rust instead of
//!   `sidechaincompress`, which is fragile across ffmpeg builds.
//!
//! ffmpeg is still used to *decode* the inputs (mp3 / m4a → PCM WAV),
//! because pulling in a full mp3 decoder dep just to round-trip 30 seconds
//! of narration is overkill. The mixing itself runs in Rust.
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

// ── Phase 5: Rust-side narration mix ──────────────────────────────────────

/// Decode any audio file (mp3 / wav / m4a / aac / opus / video container) to
/// 48kHz stereo PCM s16le WAV via ffmpeg. The output WAV stays open with
/// `hound` for the mixer.
async fn decode_to_wav(input: &Path, output: &Path) -> Result<(), NarratorError> {
    let ffmpeg = video_engine::detect_ffmpeg()?;
    let out = Command::new(ffmpeg.as_os_str())
        .no_window()
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(input.as_os_str())
        .args(["-vn", "-c:a", "pcm_s16le", "-ar", "48000", "-ac", "2"])
        .arg(output.as_os_str())
        .output()
        .await
        .map_err(|e| NarratorError::FfmpegFailed(format!("audio decode: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(NarratorError::FfmpegFailed(format!(
            "audio decode failed: {}",
            &stderr[stderr.len().saturating_sub(400)..]
        )));
    }
    Ok(())
}

/// Read every sample of a stereo s16 WAV into an interleaved f32 buffer
/// in [-1.0, 1.0]. Mono inputs are duplicated to both channels.
fn read_wav_to_f32(path: &Path) -> Result<(Vec<f32>, u32), NarratorError> {
    let mut reader = hound::WavReader::open(path)
        .map_err(|e| NarratorError::FfmpegFailed(format!("wav open: {e}")))?;
    let spec = reader.spec();
    let sr = spec.sample_rate;
    let channels = spec.channels as usize;
    let scale = 1.0_f32 / (i16::MAX as f32);
    let raw: Vec<i16> = reader
        .samples::<i16>()
        .collect::<Result<_, _>>()
        .map_err(|e| NarratorError::FfmpegFailed(format!("wav read: {e}")))?;

    let mut out = Vec::with_capacity(raw.len().max(1) * 2 / channels.max(1));
    if channels == 1 {
        for s in &raw {
            let v = (*s as f32) * scale;
            out.push(v);
            out.push(v);
        }
    } else {
        // Take left + right; if there are >2 channels, fold extras into L/R
        // by averaging — typical for surround inputs.
        let mut i = 0;
        while i + channels <= raw.len() {
            let l = (raw[i] as f32) * scale;
            let r = (raw[i + 1] as f32) * scale;
            if channels == 2 {
                out.push(l);
                out.push(r);
            } else {
                let mut sum_l = l;
                let mut sum_r = r;
                let mut nl = 1.0;
                let mut nr = 1.0;
                for c in 2..channels {
                    let v = (raw[i + c] as f32) * scale;
                    if c.is_multiple_of(2) {
                        sum_l += v;
                        nl += 1.0;
                    } else {
                        sum_r += v;
                        nr += 1.0;
                    }
                }
                out.push(sum_l / nl);
                out.push(sum_r / nr);
            }
            i += channels;
        }
    }
    Ok((out, sr))
}

fn write_wav_from_f32(path: &Path, samples: &[f32], sr: u32) -> Result<(), NarratorError> {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: sr,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)
        .map_err(|e| NarratorError::FfmpegFailed(format!("wav create: {e}")))?;
    for &s in samples {
        // Soft clip to avoid wrap-around at extremes.
        let clipped = s.clamp(-1.0, 1.0);
        let v = (clipped * (i16::MAX as f32 - 1.0)) as i16;
        writer
            .write_sample(v)
            .map_err(|e| NarratorError::FfmpegFailed(format!("wav write: {e}")))?;
    }
    writer
        .finalize()
        .map_err(|e| NarratorError::FfmpegFailed(format!("wav finalize: {e}")))?;
    Ok(())
}

/// Per-sample-block envelope for narration → ducking-gain on the original
/// audio. Window is in samples; threshold is RMS of the narration block in
/// f32 amplitude (0..1). Smoothing avoids audible pumping.
fn compute_ducking_envelope(
    narration: &[f32],
    block: usize,
    threshold: f32,
    duck_db: f32,
    smooth_ms: f32,
    sr: u32,
) -> Vec<f32> {
    let n_blocks = narration.len() / block.max(1).max(1);
    let mut env = Vec::with_capacity(n_blocks + 1);
    let duck_gain = 10f32.powf(duck_db / 20.0);
    let one_pole = (-1.0 / (smooth_ms * 0.001 * sr as f32 / block as f32)).exp();
    let mut current = 1.0_f32;
    for b in 0..n_blocks {
        let start = b * block;
        let end = (start + block).min(narration.len());
        let mut sum_sq = 0.0_f32;
        for i in (start..end).step_by(2) {
            // RMS over interleaved L+R; both channels considered.
            sum_sq += narration[i] * narration[i];
            if i + 1 < end {
                sum_sq += narration[i + 1] * narration[i + 1];
            }
        }
        let rms = (sum_sq / (end - start).max(1) as f32).sqrt();
        let target = if rms >= threshold { duck_gain } else { 1.0 };
        // One-pole smoothing toward target.
        current = target + (current - target) * one_pole;
        env.push(current);
    }
    env
}

/// Mix `narration` over `original` into `output`. Both inputs may be any
/// audio format ffmpeg understands; both are first decoded to PCM WAV
/// (48kHz stereo s16le) on disk, then mixed in Rust with optional ducking.
///
/// `narration_gain` and `original_gain` are constant linear gains applied
/// before mixing. `duck_db` controls auto-ducking strength (e.g. -10.0
/// drops the original audio by 10dB whenever the narration is non-silent);
/// pass `0.0` to disable.
pub async fn mix_narration(
    original: &Path,
    narration: &Path,
    output: &Path,
    narration_gain: f32,
    original_gain: f32,
    duck_db: f32,
) -> Result<(), NarratorError> {
    // Round-trip both to PCM WAV via ffmpeg (decode-only).
    let tmp_dir = std::env::temp_dir();
    let orig_wav = tmp_dir.join(format!("_mix_orig_{}.wav", uuid::Uuid::new_v4()));
    let narr_wav = tmp_dir.join(format!("_mix_narr_{}.wav", uuid::Uuid::new_v4()));
    let _cleanup = ScopedRemove(vec![orig_wav.clone(), narr_wav.clone()]);

    decode_to_wav(original, &orig_wav).await?;
    decode_to_wav(narration, &narr_wav).await?;

    let (orig, sr_o) = read_wav_to_f32(&orig_wav)?;
    let (narr, sr_n) = read_wav_to_f32(&narr_wav)?;
    if sr_o != sr_n {
        return Err(NarratorError::FfmpegFailed(format!(
            "sample rate mismatch after decode: {sr_o} vs {sr_n}"
        )));
    }
    let sr = sr_o;

    // Build a ducking envelope from the narration if requested. One value
    // per ~10ms block, then linearly upsampled when applied.
    let block = (sr as usize / 50).max(64) * 2; // ~20ms stereo
    let envelope = if duck_db.abs() > 0.05 {
        compute_ducking_envelope(&narr, block, 0.005, duck_db, 60.0, sr)
    } else {
        Vec::new()
    };

    let n_out = orig.len().max(narr.len());
    let mut out = Vec::with_capacity(n_out);
    for i in 0..n_out {
        let o = orig.get(i).copied().unwrap_or(0.0);
        let n = narr.get(i).copied().unwrap_or(0.0);
        let env = if envelope.is_empty() {
            1.0
        } else {
            // Upsample: nearest-block value.
            let bi = (i / block).min(envelope.len() - 1);
            envelope[bi]
        };
        out.push(o * original_gain * env + n * narration_gain);
    }

    write_wav_from_f32(output, &out, sr)?;
    Ok(())
}

/// RAII guard that removes paths on drop. Used by the mixer to clean up
/// temp WAVs even on early return.
struct ScopedRemove(Vec<PathBuf>);
impl Drop for ScopedRemove {
    fn drop(&mut self) {
        for p in &self.0 {
            let _ = std::fs::remove_file(p);
        }
    }
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
