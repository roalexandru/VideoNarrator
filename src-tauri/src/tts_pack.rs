//! Shared narration-packing logic: given per-segment TTS audio files already
//! on disk, produce a single MP3 whose timeline tracks the script — speeding
//! up segments that overrun their window (atempo, capped at COMPRESSION_CAP)
//! and inserting silence gaps where segments don't touch.
//!
//! Extracted from `commands::generate_tts` so `narrator-cli tts narrate` can
//! reuse the exact same compression behavior in tests.

use crate::error::NarratorError;
use crate::process_utils::CommandNoWindow;
use crate::video_engine;
use std::path::{Path, PathBuf};
use tokio::process::Command;

/// One TTS segment on disk: the audio file + the scripted time window it
/// should land in. `start_seconds` and `end_seconds` come from the segment's
/// slot in the narration timeline — NOT from the actual TTS audio duration.
#[derive(Debug, Clone)]
pub struct SegmentFile {
    pub index: usize,
    pub path: PathBuf,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

/// Summary of what the compression pass did. Returned so callers can emit
/// telemetry and integration tests can assert the fix landed.
///
/// `actual_timings` is the per-segment landing record: where each rendered
/// segment actually started and ended in the concatenated output after
/// accounting for overruns and compression. The GUI's burn-subtitles pass
/// consumes this via a `narration_timings.json` sidecar so SRT timings
/// track the real audio instead of the scripted plan.
#[derive(Debug, Clone, Default)]
pub struct NarrationConcatStats {
    pub segments_total: usize,
    pub segments_compressed: usize,
    pub segments_over_cap: usize,
    pub actual_timings: Vec<crate::export_engine::ActualTiming>,
}

/// Fade duration applied to the head and tail of each TTS segment before
/// concat, to suppress the clicks/pops that show up at splice boundaries when
/// MP3 frames start/end on non-zero samples.
const FADE_SECONDS: f64 = 0.03;

/// Build the per-segment ffmpeg filter chain: optional atempo compression
/// followed by afade in/out. Pure function so it can be unit-tested without
/// invoking ffmpeg.
///
/// Fade duration auto-shrinks to `effective_dur / 2` when the segment is
/// shorter than `2 × FADE_SECONDS`. Without this, the fade-in window (0..d)
/// and fade-out window (dur-d..dur) overlap and ffmpeg multiplies the two
/// envelopes in the overlap region, producing a double-attenuation dip. The
/// shrink makes fade-in finish exactly when fade-out begins, preserving the
/// "no pop at splice" property without introducing an audible dip.
fn build_segment_filter(applied_speed: f64, effective_dur: f64) -> String {
    let mut parts: Vec<String> = Vec::new();
    if applied_speed > 1.0 + f64::EPSILON {
        for f in crate::compositor::audio::atempo_factors(applied_speed) {
            parts.push(format!("atempo={f:.4}"));
        }
    }
    let fade_d = (effective_dur / 2.0).clamp(0.0, FADE_SECONDS);
    parts.push(format!("afade=t=in:d={fade_d:.3}"));
    let fade_out_start = (effective_dur - fade_d).max(0.0);
    parts.push(format!("afade=t=out:st={fade_out_start:.3}:d={fade_d:.3}"));
    parts.join(",")
}

/// Concat per-segment TTS audio files into one MP3 that tracks the script's
/// timeline. Applies atempo speed-up (capped at `speech_rate::COMPRESSION_CAP`)
/// when a segment's audio overruns its window, applies a 30ms afade in/out on
/// every segment so splice boundaries don't pop, and inserts silence where
/// the script has gaps. Temp files (_tmp_sil_*.mp3, _tmp_seg_*_proc.mp3,
/// _concat_list.txt) are written alongside the final output.
///
/// `silence_sample_rate`: the per-TTS-engine sample rate string for
/// `anullsrc=r=`. Azure's 24000 vs ElevenLabs/builtin's 44100.
pub async fn concat_narration_segments(
    segment_files: &[SegmentFile],
    video_dur: f64,
    final_path: &Path,
    temp_dir: &Path,
    silence_sample_rate: &str,
) -> Result<NarrationConcatStats, NarratorError> {
    let mut stats = NarrationConcatStats {
        segments_total: segment_files.len(),
        ..Default::default()
    };

    if segment_files.is_empty() {
        return Ok(stats);
    }

    let ffmpeg = video_engine::detect_ffmpeg().unwrap_or_else(|_| PathBuf::from("ffmpeg"));

    tracing::info!(
        "Concat-merging {} segments into {:.0}s audio",
        segment_files.len(),
        video_dur
    );

    let mut concat_parts: Vec<PathBuf> = Vec::new();
    let mut silence_idx: usize = 0;
    let mut audio_pos: f64 = 0.0;

    for sf in segment_files.iter() {
        // Silence gap before the segment, sized against the running output
        // position (not the scripted timeline) — because previous segments
        // may have overrun or been compressed, drifting the position.
        let gap = sf.start_seconds - audio_pos;
        if gap > 0.05 {
            let sil_path = temp_dir.join(format!("_tmp_sil_{silence_idx}.mp3"));
            let anullsrc = format!("anullsrc=r={silence_sample_rate}:cl=stereo");
            let _ = Command::new(ffmpeg.as_os_str())
                .no_window()
                .args(["-y", "-f", "lavfi", "-i"])
                .arg(&anullsrc)
                .args([
                    "-t",
                    &format!("{gap:.3}"),
                    "-codec:a",
                    "libmp3lame",
                    "-q:a",
                    "2",
                ])
                .arg(sil_path.as_os_str())
                .output()
                .await;
            concat_parts.push(sil_path);
            audio_pos += gap;
            silence_idx += 1;
        }

        // Probe actual TTS duration. Fall back to the script window if the
        // file is unreadable — atempo would be a no-op there anyway.
        let seg_dur = video_engine::probe_duration(sf.path.as_path())
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(
                    "Could not probe TTS segment duration: {e}, estimating from time window"
                );
                (sf.end_seconds - sf.start_seconds).max(0.5)
            });

        // Record where this segment actually starts in the output audio.
        // Computed before the compression branch so both paths share the
        // same landing position — the effective duration below decides how
        // far audio_pos advances afterwards.
        let seg_start_in_output = audio_pos;

        let window = (sf.end_seconds - sf.start_seconds).max(0.5);
        let needs_compression = seg_dur > window + 0.10;
        let applied_speed = if needs_compression {
            let ideal_speed = seg_dur / window;
            if ideal_speed > crate::speech_rate::COMPRESSION_CAP {
                stats.segments_over_cap += 1;
                tracing::warn!(
                    "Segment {} needs {:.2}× speed-up to fit but cap is {:.2}× — \
                     {:.2}s of residual overflow will be absorbed by video padding",
                    sf.index,
                    ideal_speed,
                    crate::speech_rate::COMPRESSION_CAP,
                    seg_dur - window * crate::speech_rate::COMPRESSION_CAP
                );
                crate::speech_rate::COMPRESSION_CAP
            } else {
                tracing::info!(
                    "Segment {} compressed by {:.2}× ({:.2}s → {:.2}s)",
                    sf.index,
                    ideal_speed,
                    seg_dur,
                    seg_dur / ideal_speed
                );
                ideal_speed
            }
        } else {
            1.0
        };

        let effective_dur = seg_dur / applied_speed;
        let filter_chain = build_segment_filter(applied_speed, effective_dur);
        let proc_path = temp_dir.join(format!("_tmp_seg_{:03}_proc.mp3", sf.index));
        // `-q:a 0` gives the highest-quality libmp3lame VBR (~245 kbps).
        // Upstream providers ship 128–160 kbps MP3, so `-q:a 0` keeps the
        // re-encode headroom well above source quality and minimizes the
        // generational-loss cost of this per-segment pass. `-ar` pins the
        // sample rate to match silence inserts so concat demuxer doesn't
        // hit rate-mismatches.
        let proc_output = Command::new(ffmpeg.as_os_str())
            .no_window()
            .args(["-y", "-i"])
            .arg(sf.path.as_os_str())
            .args([
                "-filter:a",
                &filter_chain,
                "-codec:a",
                "libmp3lame",
                "-q:a",
                "0",
                "-ar",
                silence_sample_rate,
            ])
            .arg(proc_path.as_os_str())
            .output()
            .await;
        // Fall back to the raw segment on any failure: we'd rather ship a
        // popping splice than drop the segment entirely.
        let (push_path, effective_dur) = match proc_output {
            Ok(o) if o.status.success() => {
                if needs_compression {
                    stats.segments_compressed += 1;
                }
                (proc_path, effective_dur)
            }
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                tracing::warn!(
                    "segment {} filter pass failed, using original (no fade): {}",
                    sf.index,
                    &stderr[..stderr.len().min(300)]
                );
                (sf.path.clone(), seg_dur)
            }
            Err(e) => {
                tracing::warn!(
                    "segment {} filter pass exec failed, using original: {e}",
                    sf.index
                );
                (sf.path.clone(), seg_dur)
            }
        };

        concat_parts.push(push_path);
        audio_pos += effective_dur;

        stats
            .actual_timings
            .push(crate::export_engine::ActualTiming {
                segment_index: sf.index,
                start_seconds: seg_start_in_output,
                end_seconds: seg_start_in_output + effective_dur,
            });
    }

    if stats.segments_compressed > 0 || stats.segments_over_cap > 0 {
        tracing::info!(
            "TTS compression: {} compressed, {} still over cap (of {} total)",
            stats.segments_compressed,
            stats.segments_over_cap,
            stats.segments_total
        );
    }

    // Trailing silence up to the scripted video duration. Only applies if the
    // last segment's scripted end is short of the video; if segments have
    // already driven audio_pos past video_dur, no silence is added.
    let last_end = segment_files.last().map(|sf| sf.end_seconds).unwrap_or(0.0);
    if video_dur > last_end + 0.1 {
        let trail = video_dur - last_end;
        let sil_path = temp_dir.join(format!("_tmp_sil_{silence_idx}.mp3"));
        let anullsrc = format!("anullsrc=r={silence_sample_rate}:cl=stereo");
        let _ = Command::new(ffmpeg.as_os_str())
            .no_window()
            .args(["-y", "-f", "lavfi", "-i"])
            .arg(&anullsrc)
            .args([
                "-t",
                &format!("{trail:.3}"),
                "-codec:a",
                "libmp3lame",
                "-q:a",
                "2",
            ])
            .arg(sil_path.as_os_str())
            .output()
            .await;
        concat_parts.push(sil_path);
    }

    // Build the concat-demuxer list file. `file '...'` entries must be
    // shell-escaped for single quotes, backslashes, and newlines.
    let concat_list_path = temp_dir.join("_concat_list.txt");
    let concat_content: String = concat_parts
        .iter()
        .map(|p| {
            let escaped = p
                .to_string_lossy()
                .replace('\\', "\\\\")
                .replace(['\n', '\r'], "")
                .replace('\'', "'\\''");
            format!("file '{escaped}'")
        })
        .collect::<Vec<_>>()
        .join("\n");
    tokio::fs::write(&concat_list_path, &concat_content).await?;

    let concat_output = Command::new(ffmpeg.as_os_str())
        .no_window()
        .args(["-y", "-f", "concat", "-safe", "0", "-i"])
        .arg(concat_list_path.as_os_str())
        .args(["-codec:a", "libmp3lame", "-q:a", "2"])
        .arg(final_path.as_os_str())
        .output()
        .await
        .map_err(|e| NarratorError::FfmpegFailed(format!("concat exec: {e}")))?;

    if !concat_output.status.success() {
        let stderr = String::from_utf8_lossy(&concat_output.stderr);
        return Err(NarratorError::FfmpegFailed(format!(
            "ffmpeg concat failed: {}",
            &stderr[stderr.len().saturating_sub(500)..]
        )));
    }

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_pass_through_is_fade_only() {
        let chain = build_segment_filter(1.0, 1.0);
        assert!(!chain.contains("atempo"));
        assert!(chain.contains("afade=t=in:d=0.030"));
        assert!(chain.contains("afade=t=out:st=0.970:d=0.030"));
    }

    #[test]
    fn filter_with_compression_includes_atempo() {
        let chain = build_segment_filter(1.5, 2.0);
        assert!(chain.contains("atempo=1.5000"));
        assert!(chain.contains("afade=t=in:d=0.030"));
        assert!(chain.contains("afade=t=out:st=1.970:d=0.030"));
    }

    #[test]
    fn filter_fade_shrinks_to_half_segment_when_too_short() {
        // For a 10ms segment, full 30ms fades would overlap and multiply to
        // a double-attenuation dip. Fade duration should halve to 5ms and
        // fade-out should start exactly where fade-in ends.
        let chain = build_segment_filter(1.0, 0.01);
        assert!(chain.contains("afade=t=in:d=0.005"));
        assert!(chain.contains("afade=t=out:st=0.005:d=0.005"));
    }

    #[test]
    fn filter_fade_shrinks_at_overlap_boundary() {
        // At exactly 2×FADE_SECONDS (60ms), fade-in and fade-out should just
        // touch at the midpoint with the full FADE_SECONDS duration.
        let chain = build_segment_filter(1.0, 0.060);
        assert!(chain.contains("afade=t=in:d=0.030"));
        assert!(chain.contains("afade=t=out:st=0.030:d=0.030"));
    }

    #[test]
    fn filter_fade_inside_overlap_window_halves_duration() {
        // A 40ms segment would otherwise overlap fade windows by 20ms.
        // Shrunk fade should be 20ms with fade-out starting at 20ms.
        let chain = build_segment_filter(1.0, 0.040);
        assert!(chain.contains("afade=t=in:d=0.020"));
        assert!(chain.contains("afade=t=out:st=0.020:d=0.020"));
    }

    #[test]
    fn filter_compression_cap_produces_single_atempo() {
        let chain = build_segment_filter(crate::speech_rate::COMPRESSION_CAP, 1.0);
        let atempo_count = chain.matches("atempo=").count();
        assert_eq!(atempo_count, 1);
    }
}
