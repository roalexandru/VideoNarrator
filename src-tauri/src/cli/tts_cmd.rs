//! `narrator-cli tts …` — headless narration audio generation.
//!
//! Uses the OS-native builtin TTS (macOS `say`, Windows PowerShell, Linux
//! espeak) so this flow runs without any API key. Exists primarily to let
//! integration tests drive the full narration-packing path (per-segment TTS +
//! atempo compression + silence gaps) that `commands::generate_tts` uses
//! internally.

use clap::Subcommand;
use serde_json::{json, Value};
use std::path::PathBuf;

use crate::builtin_tts;
use crate::error::NarratorError;
use crate::models::NarrationScript;
use crate::tts_pack::{concat_narration_segments, SegmentFile};

use super::read_json_arg;

#[derive(Subcommand, Debug)]
pub enum TtsCmd {
    /// Render a script to a single MP3 using the builtin TTS engine.
    /// This exercises the same concat + atempo-compression pipeline as the
    /// GUI's Export flow, but without requiring API keys.
    Narrate {
        /// Path to a NarrationScript JSON file, or `-` for stdin.
        #[arg(long)]
        script: String,
        /// Output MP3 path.
        #[arg(long)]
        output: String,
        /// Target video duration in seconds. Trailing silence is added up to
        /// this duration (matching how Export builds narration_full.mp3).
        /// If omitted, uses the script's `total_duration_seconds`.
        #[arg(long)]
        video_duration: Option<f64>,
        /// Builtin voice name (OS-specific). Leave empty for the OS default.
        #[arg(long, default_value = "")]
        voice: String,
        /// Playback speed multiplier for the builtin TTS (1.0 = natural).
        #[arg(long, default_value_t = 1.0)]
        speed: f32,
    },
}

pub async fn run(cmd: TtsCmd) -> Result<Value, NarratorError> {
    match cmd {
        TtsCmd::Narrate {
            script,
            output,
            video_duration,
            voice,
            speed,
        } => {
            let script: NarrationScript = read_json_arg(&script)?;
            let output_path = PathBuf::from(output);
            let out_dir = output_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."));
            tokio::fs::create_dir_all(&out_dir).await?;

            // Synthesize each segment in sequence. Parallelizing doesn't save
            // much here because `say` serializes access to the audio device
            // anyway, and deterministic ordering makes the test output stable.
            let mut segment_files: Vec<SegmentFile> = Vec::new();
            for seg in &script.segments {
                let seg_path = out_dir.join(format!("_tmp_seg_{:03}.mp3", seg.index));
                builtin_tts::generate_speech(&seg.text, &voice, speed, &seg_path).await?;
                segment_files.push(SegmentFile {
                    index: seg.index,
                    path: seg_path,
                    start_seconds: seg.start_seconds,
                    end_seconds: seg.end_seconds,
                });
            }

            let video_dur = video_duration.unwrap_or(script.total_duration_seconds);
            let stats = concat_narration_segments(
                &segment_files,
                video_dur,
                &output_path,
                &out_dir,
                "44100",
            )
            .await?;

            // Clean up per-segment temp files (keep only the final mp3 and any
            // _fast variants atempo produced, which the concat demuxer already
            // read). The test harness can choose to keep them by inspecting
            // the out dir before this runs.
            for sf in &segment_files {
                let _ = tokio::fs::remove_file(&sf.path).await;
            }
            if let Ok(mut entries) = tokio::fs::read_dir(&out_dir).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    // Two exact prefixes so a user who names their output
                    // `_tmp_anything.mp3` doesn't have it deleted out from
                    // under them. Atempo temps are `_tmp_seg_NNN_fast.mp3`,
                    // so `_tmp_seg_` catches them too.
                    if name.starts_with("_tmp_sil_")
                        || name.starts_with("_tmp_seg_")
                        || name == "_concat_list.txt"
                    {
                        let _ = tokio::fs::remove_file(entry.path()).await;
                    }
                }
            }

            Ok(json!({
                "output_path": output_path.to_string_lossy(),
                "segments_total": stats.segments_total,
                "segments_compressed": stats.segments_compressed,
                "segments_over_cap": stats.segments_over_cap,
            }))
        }
    }
}
