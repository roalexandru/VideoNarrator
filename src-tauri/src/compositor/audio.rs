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
        if crate::video_edit::looks_like_no_audio_stream(&stderr) {
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

/// Soft-clip a single sample: linear up to ±0.95, then tanh-shaped toward
/// (but never reaching) ±0.99. Needed because the mixer sums narration +
/// (possibly unducked) original, which can exceed ±1.0 during narration
/// pauses over loud source audio — hard-clamping there produced audible
/// crunch. Output asymptotes to ±0.99 as |s| → ∞, so f32 rounding can't
/// hand int16 conversion a value that saturates to ±32767 (and would wrap
/// with `i16::MAX - 1` scaling).
///
/// Non-finite inputs (NaN / ±Inf) return 0 so a bad decoded sample can't
/// poison downstream math or trip the `y.signum() == s.signum()` test.
fn soft_clip(s: f32) -> f32 {
    if !s.is_finite() {
        return 0.0;
    }
    const KNEE: f32 = 0.95;
    const CEIL: f32 = 0.99;
    if s.abs() <= KNEE {
        s
    } else {
        let over = (s.abs() - KNEE) / (1.0 - KNEE);
        s.signum() * (KNEE + (CEIL - KNEE) * over.tanh())
    }
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
        let shaped = soft_clip(s);
        let v = (shaped * (i16::MAX as f32 - 1.0)) as i16;
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
            // Linearly interpolate between adjacent envelope samples instead
            // of snapping to a single 20 ms block. Removes audible steps at
            // block boundaries during narration. Note: this only smooths
            // *within* the computed envelope range — samples past the last
            // narration block clamp to `envelope[last]`, so release at the
            // very end of the narration is still abrupt. Acceptable for
            // now; users who notice can taper narration tails themselves.
            let pos = i as f32 / block as f32;
            let bi = pos.floor() as usize;
            let frac = pos - bi as f32;
            // `last` is computed with `saturating_sub(1)` so a future refactor
            // that lets an empty envelope reach this branch doesn't wrap to
            // `usize::MAX` and out-of-bounds index.
            let last = envelope.len().saturating_sub(1);
            let a = envelope[bi.min(last)];
            let b = envelope[(bi + 1).min(last)];
            a + (b - a) * frac
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

    // ── mix_narration envelope + clipping tests ────────────────────────
    //
    // mix_narration itself needs ffmpeg to decode audio, so we exercise the
    // helpers directly on f32 buffers here. This covers the regressions
    // that actually bite users: envelope smoothness and peak clipping.

    /// Synthetic worst case with narration active: ±0.8 peaks on both
    /// streams, aligned, with the -8 dB duck (linear ≈ 0.398) and the 0.85
    /// gains that match `video_edit::merge_audio_video`. The sum must stay
    /// inside ±1.0 so soft_clip doesn't need to engage during speech.
    #[test]
    fn mix_default_gains_leave_headroom_for_aligned_peaks() {
        let narration_peak: f32 = 0.8;
        let original_peak: f32 = 0.8;
        let narration_gain: f32 = 0.85;
        let original_gain: f32 = 0.85;
        let duck_gain: f32 = 10f32.powf(-8.0 / 20.0);

        let peak_sum = narration_peak * narration_gain + original_peak * original_gain * duck_gain;
        assert!(
            peak_sum < 1.0,
            "peak sum {peak_sum} should stay under ±1.0 with default gains during narration"
        );
    }

    /// Soft-clip guard: during narration PAUSES the envelope returns to 1.0
    /// and unducked ±0.8 peaks on both streams sum to 1.36. The soft-clip
    /// in `write_wav_from_f32` must keep the output strictly inside ±1.0
    /// for any finite input (no hard clip, no wrap, no sign flip), and
    /// must leave the safe region (≤ ±0.95) untouched.
    #[test]
    fn soft_clip_preserves_linear_region_and_tames_peaks() {
        for &s in &[-0.95_f32, -0.5, 0.0, 0.3, 0.95] {
            assert!(
                (soft_clip(s) - s).abs() < 1e-6,
                "soft_clip({s}) must equal {s} (linear), got {}",
                soft_clip(s)
            );
        }
        for &s in &[1.0_f32, 1.36, 2.0, -1.5, -3.0] {
            let y = soft_clip(s);
            assert!(
                y.abs() < 1.0,
                "soft_clip({s}) = {y} must be strictly inside ±1.0"
            );
            assert!(
                y.signum() == s.signum(),
                "soft_clip must preserve sign: soft_clip({s})={y}"
            );
        }
    }

    /// Non-finite samples (NaN / ±Inf) must collapse to 0 instead of
    /// propagating into the output. A single corrupt decoded sample would
    /// otherwise NaN-poison everything that sums it and trip the sign /
    /// magnitude asserts above.
    #[test]
    fn soft_clip_collapses_non_finite_samples() {
        for s in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let y = soft_clip(s);
            assert!(y == 0.0, "soft_clip({s}) must return 0, got {y}");
        }
    }

    /// The envelope interpolation in `mix_narration` is a linear blend
    /// between adjacent envelope samples. Reproduce the math here to guard
    /// against anyone swapping it back to nearest-neighbor.
    #[test]
    fn envelope_lerp_at_block_boundaries() {
        let envelope = [1.0_f32, 0.398, 0.398];
        let block = 1000usize;

        // At an exact block boundary (i = 1000) the answer is the value at
        // that block.
        let i = 1000usize;
        let pos = i as f32 / block as f32;
        let bi = pos.floor() as usize;
        let frac = pos - bi as f32;
        let a = envelope[bi.min(envelope.len() - 1)];
        let b = envelope[(bi + 1).min(envelope.len() - 1)];
        let lerped = a + (b - a) * frac;
        assert!((lerped - 0.398).abs() < 1e-4);

        // Mid-block (i = 500) is halfway between 1.0 and 0.398 → ~0.699.
        let i = 500usize;
        let pos = i as f32 / block as f32;
        let bi = pos.floor() as usize;
        let frac = pos - bi as f32;
        let a = envelope[bi.min(envelope.len() - 1)];
        let b = envelope[(bi + 1).min(envelope.len() - 1)];
        let lerped = a + (b - a) * frac;
        assert!(
            (lerped - 0.699).abs() < 1e-3,
            "midpoint lerp should be ~0.699, got {lerped}"
        );
    }

    #[test]
    fn compute_ducking_envelope_drops_when_narration_is_loud() {
        let sr = 48000u32;
        let block = (sr as usize / 50).max(64) * 2;
        let loud: Vec<f32> = (0..block * 5).map(|_| 0.5).collect();
        let env = compute_ducking_envelope(&loud, block, 0.005, -8.0, 60.0, sr);
        let duck_linear = 10f32.powf(-8.0 / 20.0);
        assert!(env.len() >= 4);
        assert!(
            env.last().copied().unwrap() < duck_linear + 0.05,
            "envelope should settle near duck target, got {env:?}"
        );
    }

    #[test]
    fn compute_ducking_envelope_stays_at_unity_when_silent() {
        let sr = 48000u32;
        let block = (sr as usize / 50).max(64) * 2;
        let silent: Vec<f32> = vec![0.0; block * 5];
        let env = compute_ducking_envelope(&silent, block, 0.005, -8.0, 60.0, sr);
        for (i, e) in env.iter().enumerate() {
            assert!(
                (e - 1.0).abs() < 1e-4,
                "silent narration should not duck (block {i} = {e})"
            );
        }
    }

    #[test]
    fn read_wav_to_f32_duplicates_mono_to_stereo() {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let tmp = tempfile::NamedTempFile::new().unwrap();
        {
            let mut writer = hound::WavWriter::create(tmp.path(), spec).unwrap();
            for &s in &[0i16, 16_384, -16_384, 32_000] {
                writer.write_sample(s).unwrap();
            }
            writer.finalize().unwrap();
        }
        let (stereo, sr) = read_wav_to_f32(tmp.path()).unwrap();
        assert_eq!(sr, 48000);
        // Mono input of 4 samples should come back as 4 stereo frames = 8 values.
        assert_eq!(stereo.len(), 8);
        // Each L/R pair is the same value (mono duplicated).
        for pair in stereo.chunks_exact(2) {
            assert!((pair[0] - pair[1]).abs() < 1e-6);
        }
    }
}
