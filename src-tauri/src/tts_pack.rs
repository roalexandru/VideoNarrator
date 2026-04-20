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

/// Concat per-segment TTS audio files into one MP3 that tracks the script's
/// timeline. Applies atempo speed-up (capped at `speech_rate::COMPRESSION_CAP`)
/// when a segment's audio overruns its window, and inserts silence where the
/// script has gaps. Temp files (_tmp_sil_*.mp3, _tmp_seg_*_fast.mp3,
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
        let (push_path, effective_dur) = if seg_dur > window + 0.10 {
            let ideal_speed = seg_dur / window;
            let applied_speed = ideal_speed.min(crate::speech_rate::COMPRESSION_CAP);
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
            } else {
                tracing::info!(
                    "Segment {} compressed by {:.2}× ({:.2}s → {:.2}s)",
                    sf.index,
                    applied_speed,
                    seg_dur,
                    seg_dur / applied_speed
                );
            }
            let factors = crate::compositor::audio::atempo_factors(applied_speed);
            debug_assert!(!factors.is_empty());
            let filter_chain: String = factors
                .iter()
                .map(|f| format!("atempo={f:.4}"))
                .collect::<Vec<_>>()
                .join(",");
            let fast_path = temp_dir.join(format!("_tmp_seg_{:03}_fast.mp3", sf.index));
            let fast_output = Command::new(ffmpeg.as_os_str())
                .no_window()
                .args(["-y", "-i"])
                .arg(sf.path.as_os_str())
                .args([
                    "-filter:a",
                    &filter_chain,
                    "-codec:a",
                    "libmp3lame",
                    "-q:a",
                    "2",
                ])
                .arg(fast_path.as_os_str())
                .output()
                .await;
            match fast_output {
                Ok(o) if o.status.success() => {
                    stats.segments_compressed += 1;
                    (fast_path, seg_dur / applied_speed)
                }
                Ok(o) => {
                    let stderr = String::from_utf8_lossy(&o.stderr);
                    tracing::warn!(
                        "atempo compression failed for segment {}, using original: {}",
                        sf.index,
                        &stderr[..stderr.len().min(300)]
                    );
                    (sf.path.clone(), seg_dur)
                }
                Err(e) => {
                    tracing::warn!(
                        "atempo compression exec failed for segment {}: {e}",
                        sf.index
                    );
                    (sf.path.clone(), seg_dur)
                }
            }
        } else {
            (sf.path.clone(), seg_dur)
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
