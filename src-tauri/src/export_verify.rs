//! Post-export verification: does the rendered file match what we planned?
//!
//! The critique pass checks the *script*. Nothing checked the *artifact*. Every
//! render step reports success on a zero ffmpeg exit code, which is not the same
//! as "the output is what the user asked for" — the burn-in path is the
//! motivating example. `CLAUDE.md` documents that an ffmpeg without libass
//! "silently breaks burn-subtitles export"; `ffmpeg_supports_subtitle_burn` is a
//! *pre*-check, and only a post-check proves the captions reached the pixels.
//!
//! Each check compares the delivered file against a property we intended:
//!
//! | Check | Intent it verifies |
//! |---|---|
//! | duration | the timeline was not truncated or stretched |
//! | `bt709` transfer | HDR tone mapping ran |
//! | faststart | the file streams progressively |
//! | integrated loudness | the mix hit the streaming target |
//! | audio stream present | narration actually got muxed |
//! | subtitle burn | captions changed the pixels |
//!
//! Advisory, never blocking. A failed check is information for the user, not a
//! reason to withhold a file they just waited for — and a check that cannot run
//! (no ffprobe, unreadable file) reports `Skipped` rather than failing.

use crate::error::NarratorError;
use crate::loudness;
use crate::video_engine;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Outcome of one check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "status", content = "detail")]
pub enum CheckStatus {
    Pass(String),
    /// The output does not match what we planned. Advisory.
    Fail(String),
    /// Could not be determined — a missing tool or an inapplicable export.
    Skipped(String),
}

impl CheckStatus {
    pub fn is_fail(&self) -> bool {
        matches!(self, CheckStatus::Fail(_))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Check {
    /// Stable identifier the UI can key off.
    pub id: String,
    /// Short human-readable label.
    pub label: String,
    #[serde(flatten)]
    pub status: CheckStatus,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VerificationReport {
    pub checks: Vec<Check>,
}

impl VerificationReport {
    pub fn failures(&self) -> usize {
        self.checks.iter().filter(|c| c.status.is_fail()).count()
    }

    /// True when nothing failed. `Skipped` does not count as a failure — an
    /// inapplicable check says nothing about the output.
    #[cfg(test)]
    pub fn all_passed(&self) -> bool {
        self.failures() == 0
    }

    fn push(&mut self, id: &str, label: &str, status: CheckStatus) {
        self.checks.push(Check {
            id: id.to_string(),
            label: label.to_string(),
            status,
        });
    }
}

/// Tolerance for the duration check, in seconds.
///
/// Generous on purpose: `-c:v copy` keeps whole GOPs, the trailing-silence pad
/// is sized in whole frames, and container duration is rounded. A real
/// truncation is seconds wrong, not tenths.
pub const DURATION_TOLERANCE_SECS: f64 = 1.5;

/// Tolerance for the loudness check, in LU.
///
/// `loudnorm` targets the integrated value but a true-peak limit can hold it
/// back on very dynamic material, so this allows more slack than the
/// measurement precision would suggest.
pub const LOUDNESS_TOLERANCE_LU: f64 = 1.5;

/// Compare a measured duration against the script's plan.
pub fn check_duration(measured: f64, expected: f64) -> CheckStatus {
    if expected <= 0.0 {
        return CheckStatus::Skipped("script has no duration to compare".into());
    }
    let delta = measured - expected;
    if delta.abs() <= DURATION_TOLERANCE_SECS {
        CheckStatus::Pass(format!("{measured:.1}s (planned {expected:.1}s)"))
    } else {
        CheckStatus::Fail(format!(
            "{measured:.1}s but the script plans {expected:.1}s ({:+.1}s)",
            delta
        ))
    }
}

/// Verify the delivered file is SDR Rec.709 rather than still-tagged HDR.
pub fn check_color_transfer(transfer: Option<&str>) -> CheckStatus {
    match transfer {
        // Most SDR files simply omit the tag; absence is not a defect.
        None => CheckStatus::Pass("SDR (no transfer tag)".into()),
        Some(t) if video_engine::is_hdr_transfer(t) => CheckStatus::Fail(format!(
            "still tagged {t} — HDR tone mapping did not run, colours will look washed out"
        )),
        Some(t) => CheckStatus::Pass(t.to_string()),
    }
}

/// Verify `moov` precedes `mdat`, i.e. `+faststart` was applied.
///
/// Read from the bytes rather than trusting that a flag was passed: ffprobe
/// exposes no field for this, and the byte order *is* the property a streaming
/// player depends on.
pub fn check_faststart(bytes: &[u8]) -> CheckStatus {
    let find = |needle: &[u8]| bytes.windows(needle.len()).position(|w| w == needle);
    match (find(b"moov"), find(b"mdat")) {
        (Some(moov), Some(mdat)) if moov < mdat => CheckStatus::Pass("moov precedes mdat".into()),
        (Some(_), Some(_)) => CheckStatus::Fail(
            "moov comes after mdat — players must download the whole file before playing".into(),
        ),
        _ => CheckStatus::Skipped("not an MP4 container".into()),
    }
}

/// Compare measured integrated loudness against the streaming target.
pub fn check_loudness(measured_i: Option<f64>) -> CheckStatus {
    match measured_i {
        None => CheckStatus::Skipped("could not measure loudness".into()),
        Some(i) if !i.is_finite() => CheckStatus::Skipped("track is silent".into()),
        Some(i) if (i - loudness::TARGET_I).abs() <= LOUDNESS_TOLERANCE_LU => {
            CheckStatus::Pass(format!("{i:.1} LUFS (target {:.0})", loudness::TARGET_I))
        }
        Some(i) => CheckStatus::Fail(format!(
            "{i:.1} LUFS, off target {:.0} by {:+.1} LU — platforms will play this \
             at a different level than surrounding content",
            loudness::TARGET_I,
            i - loudness::TARGET_I
        )),
    }
}

/// Verify narration actually reached the container.
pub fn check_audio_present(has_audio: bool, narration_expected: bool) -> CheckStatus {
    match (narration_expected, has_audio) {
        (false, _) => CheckStatus::Skipped("export has no narration track".into()),
        (true, true) => CheckStatus::Pass("audio stream present".into()),
        (true, false) => {
            CheckStatus::Fail("no audio stream — narration did not reach the output".into())
        }
    }
}

/// Verify the subtitle burn changed the pixels.
///
/// Compares a frame sampled at a caption timestamp in the burnt output against
/// the same timestamp in the pre-burn source. libass writes opaque text, so an
/// identical frame means the filter silently did nothing — the exact failure
/// mode an ffmpeg without libass produces.
pub fn check_subtitles_burnt(before: &[u8], after: &[u8]) -> CheckStatus {
    if before.is_empty() || after.is_empty() {
        return CheckStatus::Skipped("could not sample a frame at a caption".into());
    }
    if before == after {
        CheckStatus::Fail(
            "the frame at a caption timestamp is unchanged — subtitles were not burnt in \
             (this ffmpeg may lack libass)"
                .into(),
        )
    } else {
        CheckStatus::Pass("captions are present in the rendered frames".into())
    }
}

/// Inputs describing what the export was supposed to produce.
#[derive(Debug, Clone, Default)]
pub struct ExportIntent {
    /// The script's planned total duration, in seconds.
    pub expected_duration: f64,
    /// True when a narration audio track was muxed in.
    pub narration_expected: bool,
    /// When subtitles were burnt: the pre-burn video plus a timestamp at which a
    /// caption is on screen.
    pub burn_check: Option<(std::path::PathBuf, f64)>,
}

/// Run every applicable check against `output_path`.
///
/// Never returns `Err` for a failed *check* — only for an output file that
/// cannot be probed at all, which the caller may also choose to treat as
/// advisory. One pass, no retries: this reports, it does not repair.
pub async fn verify_export(
    output_path: &Path,
    intent: &ExportIntent,
) -> Result<VerificationReport, NarratorError> {
    let mut report = VerificationReport::default();

    let metadata = video_engine::probe_video(output_path).await?;
    report.push(
        "duration",
        "Duration matches the script",
        check_duration(metadata.duration_seconds, intent.expected_duration),
    );

    let transfer = video_engine::probe_color_transfer(output_path)
        .await
        .ok()
        .flatten();
    report.push(
        "color_transfer",
        "Colour is Rec.709 SDR",
        check_color_transfer(transfer.as_deref()),
    );

    // Read only the header region: faststart is decided in the first bytes, and
    // slurping a multi-GB export to check byte order would be absurd.
    let faststart = match read_prefix(output_path, 512 * 1024).await {
        Ok(bytes) => check_faststart(&bytes),
        Err(e) => CheckStatus::Skipped(format!("could not read container header: {e}")),
    };
    report.push("faststart", "Streams progressively", faststart);

    let has_audio = video_engine::probe_has_audio_stream(output_path)
        .await
        .unwrap_or(false);
    report.push(
        "audio_present",
        "Narration track present",
        check_audio_present(has_audio, intent.narration_expected),
    );

    let loudness_status = if intent.narration_expected && has_audio {
        check_loudness(loudness::measure(output_path).await.map(|s| s.input_i))
    } else {
        CheckStatus::Skipped("no audio to measure".into())
    };
    report.push("loudness", "Loudness on target", loudness_status);

    if let Some((pre_burn, timestamp)) = &intent.burn_check {
        let status = match (
            sample_frame(pre_burn, *timestamp).await,
            sample_frame(output_path, *timestamp).await,
        ) {
            (Ok(before), Ok(after)) => check_subtitles_burnt(&before, &after),
            _ => CheckStatus::Skipped("could not sample frames to compare".into()),
        };
        report.push("subtitles_burnt", "Subtitles burnt in", status);
    }

    let failures = report.failures();
    if failures > 0 {
        tracing::warn!("export verification: {failures} check(s) did not pass");
    } else {
        tracing::info!(
            "export verification: all {} checks passed",
            report.checks.len()
        );
    }
    Ok(report)
}

/// Read at most `limit` bytes from the start of a file.
async fn read_prefix(path: &Path, limit: usize) -> Result<Vec<u8>, std::io::Error> {
    use tokio::io::AsyncReadExt;
    let file = tokio::fs::File::open(path).await?;
    let mut buf = Vec::with_capacity(limit.min(64 * 1024));
    file.take(limit as u64).read_to_end(&mut buf).await?;
    Ok(buf)
}

/// Extract one frame at `timestamp` as raw bytes, for pixel comparison.
async fn sample_frame(video: &Path, timestamp: f64) -> Result<Vec<u8>, NarratorError> {
    let out = std::env::temp_dir().join(format!("_narrator_verify_{}.png", uuid::Uuid::new_v4()));
    crate::video_edit::extract_single_frame(
        &video.to_string_lossy(),
        timestamp,
        &out.to_string_lossy(),
    )
    .await?;
    let bytes = tokio::fs::read(&out)
        .await
        .map_err(|e| NarratorError::FrameExtractionError(e.to_string()))?;
    let _ = tokio::fs::remove_file(&out).await;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_passes_within_tolerance_and_fails_outside() {
        assert!(matches!(check_duration(60.0, 60.0), CheckStatus::Pass(_)));
        // Whole-GOP copies and frame-rounded padding produce sub-second drift.
        assert!(matches!(check_duration(60.9, 60.0), CheckStatus::Pass(_)));
        assert!(matches!(check_duration(59.1, 60.0), CheckStatus::Pass(_)));
        // A real truncation is seconds wrong.
        assert!(check_duration(30.0, 60.0).is_fail());
        assert!(check_duration(120.0, 60.0).is_fail());
    }

    #[test]
    fn duration_failure_message_states_the_direction() {
        // A reviewer reading the panel needs to know which way it went.
        match check_duration(30.0, 60.0) {
            CheckStatus::Fail(msg) => {
                assert!(msg.contains("30.0"), "{msg}");
                assert!(msg.contains("60.0"), "{msg}");
                assert!(msg.contains("-30.0"), "signed delta missing: {msg}");
            }
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    #[test]
    fn duration_is_skipped_when_there_is_nothing_to_compare() {
        assert!(matches!(check_duration(60.0, 0.0), CheckStatus::Skipped(_)));
    }

    #[test]
    fn color_transfer_flags_untonemapped_hdr() {
        assert!(check_color_transfer(Some("smpte2084")).is_fail(), "PQ");
        assert!(check_color_transfer(Some("arib-std-b67")).is_fail(), "HLG");
    }

    #[test]
    fn color_transfer_accepts_sdr_and_an_absent_tag() {
        // Most SDR files omit the tag entirely — that must not read as a defect.
        assert!(matches!(check_color_transfer(None), CheckStatus::Pass(_)));
        for sdr in ["bt709", "smpte170m", "iec61966-2-1"] {
            assert!(
                matches!(check_color_transfer(Some(sdr)), CheckStatus::Pass(_)),
                "{sdr} must pass"
            );
        }
    }

    #[test]
    fn faststart_reads_atom_order_from_the_bytes() {
        let mut good = vec![0u8; 40];
        good.extend_from_slice(b"moov");
        good.extend_from_slice(&[0u8; 100]);
        good.extend_from_slice(b"mdat");
        assert!(matches!(check_faststart(&good), CheckStatus::Pass(_)));

        let mut bad = vec![0u8; 40];
        bad.extend_from_slice(b"mdat");
        bad.extend_from_slice(&[0u8; 100]);
        bad.extend_from_slice(b"moov");
        assert!(check_faststart(&bad).is_fail());
    }

    #[test]
    fn faststart_is_skipped_for_a_non_mp4() {
        assert!(matches!(
            check_faststart(b"this is not a container"),
            CheckStatus::Skipped(_)
        ));
        assert!(matches!(check_faststart(&[]), CheckStatus::Skipped(_)));
        // Only one of the two atoms present is still indeterminate.
        assert!(matches!(
            check_faststart(b"....moov...."),
            CheckStatus::Skipped(_)
        ));
    }

    #[test]
    fn loudness_passes_on_target_and_fails_off_it() {
        assert!(matches!(check_loudness(Some(-14.0)), CheckStatus::Pass(_)));
        assert!(matches!(check_loudness(Some(-15.2)), CheckStatus::Pass(_)));
        assert!(matches!(check_loudness(Some(-12.8)), CheckStatus::Pass(_)));
        // The pre-normalization case this whole feature exists to catch.
        assert!(check_loudness(Some(-23.0)).is_fail());
        assert!(check_loudness(Some(-6.0)).is_fail());
    }

    #[test]
    fn loudness_is_skipped_when_unmeasurable_or_silent() {
        assert!(matches!(check_loudness(None), CheckStatus::Skipped(_)));
        assert!(matches!(
            check_loudness(Some(f64::NEG_INFINITY)),
            CheckStatus::Skipped(_)
        ));
    }

    #[test]
    fn audio_check_only_applies_when_narration_was_expected() {
        assert!(check_audio_present(false, true).is_fail());
        assert!(matches!(
            check_audio_present(true, true),
            CheckStatus::Pass(_)
        ));
        // A subtitles-only export legitimately has no narration.
        assert!(matches!(
            check_audio_present(false, false),
            CheckStatus::Skipped(_)
        ));
    }

    #[test]
    fn subtitle_burn_fails_when_the_frame_is_unchanged() {
        // The libass silent-failure mode: filter ran, pixels identical.
        let frame = vec![1u8, 2, 3, 4, 5];
        assert!(check_subtitles_burnt(&frame, &frame).is_fail());
    }

    #[test]
    fn subtitle_burn_passes_when_pixels_changed() {
        assert!(matches!(
            check_subtitles_burnt(&[1, 2, 3], &[1, 2, 4]),
            CheckStatus::Pass(_)
        ));
    }

    #[test]
    fn subtitle_burn_is_skipped_without_samples() {
        assert!(matches!(
            check_subtitles_burnt(&[], &[1, 2, 3]),
            CheckStatus::Skipped(_)
        ));
        assert!(matches!(
            check_subtitles_burnt(&[1, 2, 3], &[]),
            CheckStatus::Skipped(_)
        ));
    }

    #[test]
    fn report_counts_only_failures() {
        let mut report = VerificationReport::default();
        report.push("a", "A", CheckStatus::Pass("fine".into()));
        report.push("b", "B", CheckStatus::Skipped("n/a".into()));
        assert_eq!(report.failures(), 0);
        assert!(report.all_passed(), "skipped must not count as failure");

        report.push("c", "C", CheckStatus::Fail("broken".into()));
        assert_eq!(report.failures(), 1);
        assert!(!report.all_passed());
    }

    #[test]
    fn report_serializes_for_the_frontend() {
        let mut report = VerificationReport::default();
        report.push("duration", "Duration", CheckStatus::Pass("60.0s".into()));
        report.push(
            "loudness",
            "Loudness",
            CheckStatus::Fail("-23.0 LUFS".into()),
        );
        let json = serde_json::to_value(&report).expect("serializes");
        let checks = json["checks"].as_array().expect("checks array");
        assert_eq!(checks[0]["id"], "duration");
        assert_eq!(checks[0]["status"], "pass");
        assert_eq!(checks[1]["status"], "fail");
        // The detail string must survive — it is what the panel shows.
        assert_eq!(checks[1]["detail"], "-23.0 LUFS");
    }
}
