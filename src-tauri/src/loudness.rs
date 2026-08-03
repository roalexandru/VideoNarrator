//! EBU R128 loudness normalization for the exported audio track.
//!
//! Narration audio comes out of whichever TTS engine the user picked, mixed
//! with whatever level the source video happened to be mastered at. Nothing in
//! the pipeline targeted an absolute loudness, so exports landed wherever that
//! arithmetic put them — typically several LU quieter than everything else in a
//! viewer's feed, because every distribution platform (YouTube, Spotify,
//! LinkedIn, Instagram) normalizes playback to roughly -14 LUFS and *attenuates*
//! anything louder while leaving quiet content quiet.
//!
//! ## Why two passes
//!
//! Single-pass `loudnorm` is a dynamic normalizer: it adapts as it goes, which
//! makes it pump audibly on speech with pauses — exactly our content. The
//! two-pass form measures the whole file first, then applies one constant
//! correction, which is linear and artifact-free.
//!
//! Pass 1 (`print_format=json`, output discarded) reports `measured_I`,
//! `measured_TP`, `measured_LRA`, `measured_thresh` and `target_offset`. Feeding
//! those back in pass 2 lets `loudnorm` compute a single gain instead of
//! guessing.
//!
//! ## Failure policy
//!
//! Every function here degrades rather than fails. A missing `loudnorm` filter,
//! an unparseable pass-1 report, or a non-zero ffmpeg exit all resolve to "no
//! normalization" — the export still completes at its original level. Shipping
//! slightly-quiet audio is recoverable; failing the export is not.

use crate::error::NarratorError;
use crate::process_utils::CommandNoWindow;
use std::path::Path;
use tokio::process::Command;

/// Integrated loudness target, in LUFS.
///
/// -14 is the de-facto streaming target (YouTube, Spotify, Amazon). Broadcast
/// (-23, EBU R128) would be markedly quieter than the platforms these videos
/// are actually posted to.
pub const TARGET_I: f64 = -14.0;

/// True-peak ceiling, in dBTP. -1 leaves headroom for the lossy-codec
/// overshoot that AAC encoding introduces after normalization.
pub const TARGET_TP: f64 = -1.0;

/// Loudness range target, in LU. 11 preserves normal speech dynamics; forcing
/// it lower flattens delivery.
pub const TARGET_LRA: f64 = 11.0;

/// Pass-1 measurements needed to make pass 2 a linear correction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoudnessStats {
    pub input_i: f64,
    pub input_tp: f64,
    pub input_lra: f64,
    pub input_thresh: f64,
    pub target_offset: f64,
}

/// Parse the JSON object `loudnorm print_format=json` writes to stderr.
///
/// ffmpeg emits the report *after* its own log lines, and the values are JSON
/// strings rather than numbers. `-inf` appears for a silent input, which is not
/// valid JSON-parseable float — such a measurement is rejected, because
/// normalizing silence would apply enormous gain to noise.
pub fn parse_loudnorm_report(stderr: &str) -> Option<LoudnessStats> {
    // Take the last `{...}` block: ffmpeg's own logging can contain braces.
    let start = stderr.rfind('{')?;
    let end = stderr[start..].find('}')? + start + 1;
    let json: serde_json::Value = serde_json::from_str(&stderr[start..end]).ok()?;

    // Values arrive as strings ("-23.5"), occasionally as numbers.
    let field = |key: &str| -> Option<f64> {
        let v = json.get(key)?;
        match v {
            serde_json::Value::String(s) => s.trim().parse::<f64>().ok(),
            serde_json::Value::Number(n) => n.as_f64(),
            _ => None,
        }
        .filter(|f| f.is_finite())
    };

    Some(LoudnessStats {
        input_i: field("input_i")?,
        input_tp: field("input_tp")?,
        input_lra: field("input_lra")?,
        input_thresh: field("input_thresh")?,
        // Absent on some builds; 0.0 is the neutral value.
        target_offset: field("target_offset").unwrap_or(0.0),
    })
}

/// The pass-1 filter: measure only, print a JSON report.
pub fn measure_filter() -> String {
    format!("loudnorm=I={TARGET_I}:TP={TARGET_TP}:LRA={TARGET_LRA}:print_format=json")
}

/// The pass-2 filter: apply a constant correction derived from `stats`.
///
/// `linear=true` asks loudnorm for a single gain rather than dynamic
/// adaptation; it silently falls back to dynamic if the requested change would
/// breach the true-peak ceiling, which is the correct trade-off.
pub fn apply_filter(stats: &LoudnessStats) -> String {
    format!(
        "loudnorm=I={TARGET_I}:TP={TARGET_TP}:LRA={TARGET_LRA}:\
         measured_I={:.2}:measured_TP={:.2}:measured_LRA={:.2}:measured_thresh={:.2}:\
         offset={:.2}:linear=true",
        stats.input_i, stats.input_tp, stats.input_lra, stats.input_thresh, stats.target_offset
    )
}

/// True if the detected ffmpeg has the `loudnorm` audio filter.
///
/// Present in every mainstream build since 3.x, including both bundled
/// sidecars — but checked rather than assumed, for the same reason `zscale` is:
/// a missing filter fails the whole export.
pub fn ffmpeg_has_loudnorm() -> bool {
    use std::sync::OnceLock;
    static CACHED: OnceLock<bool> = OnceLock::new();
    *CACHED.get_or_init(|| {
        let Ok(ffmpeg) = crate::video_engine::detect_ffmpeg() else {
            return false;
        };
        let Ok(out) = std::process::Command::new(ffmpeg.as_os_str())
            .no_window()
            .args(["-hide_banner", "-filters"])
            .output()
        else {
            return false;
        };
        if !out.status.success() {
            return false;
        }
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .any(|line| line.split_whitespace().nth(1) == Some("loudnorm"))
    })
}

/// Run pass 1 and return the measurements, or `None` if anything went wrong.
///
/// Decodes audio only (`-vn`) and discards output (`-f null`), so cost is one
/// audio decode — a few seconds even for a long video.
pub async fn measure(audio_path: &Path) -> Option<LoudnessStats> {
    let ffmpeg = crate::video_engine::detect_ffmpeg().ok()?;
    let output = Command::new(ffmpeg.as_os_str())
        .no_window()
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(audio_path.as_os_str())
        .args(["-vn", "-af", &measure_filter(), "-f", "null", "-"])
        .output()
        .await
        .ok()?;

    // loudnorm writes its report to stderr even on success.
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        tracing::warn!(
            "loudness measurement failed, leaving level unchanged: {}",
            stderr.lines().rev().take(3).collect::<Vec<_>>().join(" ")
        );
        return None;
    }
    let stats = parse_loudnorm_report(&stderr);
    if stats.is_none() {
        tracing::warn!("could not parse loudnorm report, leaving level unchanged");
    }
    stats
}

/// Two-pass normalize `audio_path` into `out_path`, as 48 kHz stereo PCM.
///
/// Returns `Ok(true)` when `out_path` was written and should be used, and
/// `Ok(false)` when normalization was skipped for any reason — in which case
/// the caller must keep using the original audio. Only propagates `Err` for a
/// failure to launch ffmpeg at all.
pub async fn normalize_to(audio_path: &Path, out_path: &Path) -> Result<bool, NarratorError> {
    if !ffmpeg_has_loudnorm() {
        tracing::warn!("this ffmpeg has no loudnorm filter — skipping loudness normalization");
        return Ok(false);
    }

    let Some(stats) = measure(audio_path).await else {
        return Ok(false);
    };

    // Already within half a LU of target: re-encoding would only cost quality.
    if (stats.input_i - TARGET_I).abs() < 0.5 {
        tracing::info!(
            "audio already at {:.1} LUFS (target {TARGET_I}) — skipping normalization",
            stats.input_i
        );
        return Ok(false);
    }

    let ffmpeg = crate::video_engine::detect_ffmpeg()?;
    let output = Command::new(ffmpeg.as_os_str())
        .no_window()
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(audio_path.as_os_str())
        .args([
            "-af",
            &apply_filter(&stats),
            // loudnorm resamples internally; pin the output format so the
            // downstream mux sees exactly what it expects.
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
        .map_err(|e| NarratorError::FfmpegFailed(format!("loudnorm apply: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!(
            "loudness normalization failed, using original audio: {}",
            stderr.lines().rev().take(3).collect::<Vec<_>>().join(" ")
        );
        let _ = tokio::fs::remove_file(out_path).await;
        return Ok(false);
    }

    tracing::info!(
        "normalized audio from {:.1} to {TARGET_I} LUFS",
        stats.input_i
    );
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Realistic pass-1 output: ffmpeg log lines, then the JSON report, with
    /// every value quoted.
    const SAMPLE_REPORT: &str = r#"
[Parsed_loudnorm_0 @ 0x14f704080]
{
	"input_i" : "-23.45",
	"input_tp" : "-6.12",
	"input_lra" : "7.30",
	"input_thresh" : "-33.87",
	"output_i" : "-14.02",
	"output_tp" : "-1.01",
	"output_lra" : "7.10",
	"output_thresh" : "-24.44",
	"normalization_type" : "dynamic",
	"target_offset" : "0.02"
}
"#;

    #[test]
    fn parses_a_real_loudnorm_report() {
        let stats = parse_loudnorm_report(SAMPLE_REPORT).expect("report must parse");
        assert_eq!(stats.input_i, -23.45);
        assert_eq!(stats.input_tp, -6.12);
        assert_eq!(stats.input_lra, 7.30);
        assert_eq!(stats.input_thresh, -33.87);
        assert_eq!(stats.target_offset, 0.02);
    }

    #[test]
    fn parses_a_report_with_numeric_values() {
        // Not all builds quote the numbers.
        let raw = r#"{"input_i": -20.0, "input_tp": -3.0, "input_lra": 5.0,
                      "input_thresh": -30.0, "target_offset": 0.0}"#;
        let stats = parse_loudnorm_report(raw).expect("numeric form must parse");
        assert_eq!(stats.input_i, -20.0);
    }

    #[test]
    fn tolerates_ffmpeg_log_noise_containing_braces() {
        let noisy = format!("[matroska @ 0x1]{{weird}} log line\n{SAMPLE_REPORT}");
        let stats = parse_loudnorm_report(&noisy).expect("must find the real report");
        assert_eq!(stats.input_i, -23.45);
    }

    #[test]
    fn rejects_a_silent_input_measurement() {
        // A silent file measures -inf. Normalizing that would apply enormous
        // gain to whatever noise floor exists.
        let silent = r#"{"input_i" : "-inf", "input_tp" : "-inf",
                         "input_lra" : "0.00", "input_thresh" : "-inf",
                         "target_offset" : "0.00"}"#;
        assert!(parse_loudnorm_report(silent).is_none());
    }

    #[test]
    fn rejects_reports_missing_required_fields() {
        assert!(parse_loudnorm_report(r#"{"input_i": "-20.0"}"#).is_none());
        assert!(parse_loudnorm_report("no json here at all").is_none());
        assert!(parse_loudnorm_report("").is_none());
        assert!(parse_loudnorm_report("{ unterminated").is_none());
    }

    #[test]
    fn measure_filter_requests_a_json_report() {
        let f = measure_filter();
        assert!(f.starts_with("loudnorm="));
        assert!(
            f.contains("print_format=json"),
            "pass 1 must be measure-only"
        );
        assert!(f.contains("I=-14"));
        assert!(f.contains("TP=-1"));
        assert!(f.contains("LRA=11"));
        // Pass 1 must not carry measurements — that is what makes it pass 1.
        assert!(!f.contains("measured_I"));
    }

    #[test]
    fn apply_filter_feeds_every_measurement_back() {
        let stats = parse_loudnorm_report(SAMPLE_REPORT).unwrap();
        let f = apply_filter(&stats);
        // All four measurements must be present, or loudnorm silently reverts
        // to dynamic mode and the pumping this design avoids comes back.
        assert!(f.contains("measured_I=-23.45"));
        assert!(f.contains("measured_TP=-6.12"));
        assert!(f.contains("measured_LRA=7.30"));
        assert!(f.contains("measured_thresh=-33.87"));
        assert!(f.contains("offset=0.02"));
        assert!(f.contains("linear=true"), "pass 2 must be a constant gain");
        // Targets must still be stated in pass 2.
        assert!(f.contains("I=-14"));
        assert!(!f.contains("print_format"), "pass 2 must not re-measure");
    }

    #[test]
    fn apply_filter_is_a_single_valid_filter_expression() {
        let stats = parse_loudnorm_report(SAMPLE_REPORT).unwrap();
        let f = apply_filter(&stats);
        // The multi-line format! string must not leak whitespace into the
        // filter — ffmpeg rejects a filter argument containing spaces.
        assert!(
            !f.contains(' ') && !f.contains('\n') && !f.contains('\t'),
            "filter must be whitespace-free, got: {f}"
        );
        assert_eq!(f.matches("loudnorm=").count(), 1);
    }

    #[test]
    fn targets_match_the_streaming_convention() {
        // These are asserted so a casual edit doesn't silently retarget every
        // future export.
        assert_eq!(TARGET_I, -14.0);
        assert_eq!(TARGET_TP, -1.0);
        assert_eq!(TARGET_LRA, 11.0);
    }

    #[tokio::test]
    async fn measuring_a_nonexistent_file_returns_none() {
        assert!(measure(Path::new("/nonexistent/none.wav")).await.is_none());
    }

    #[tokio::test]
    async fn normalizing_a_nonexistent_file_reports_skipped() {
        let out = std::env::temp_dir().join("_loudness_test_should_not_exist.wav");
        let used = normalize_to(Path::new("/nonexistent/none.wav"), &out)
            .await
            .expect("must not error, only skip");
        assert!(!used, "caller must be told to keep the original audio");
        assert!(!out.exists(), "no partial output left behind");
    }
}
