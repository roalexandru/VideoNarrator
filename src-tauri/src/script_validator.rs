//! Post-parse validation of a `NarrationScript` against the per-language speech
//! rate budget. Never mutates the script — emits a report so Review and Export
//! can act on it (show an overflow badge, compress / pad at export, or retry
//! generation with a tighter budget).

use crate::models::NarrationScript;
use crate::speech_rate::{estimate_tts_seconds, SegmentOverflow, Severity};

/// Walk all segments and classify each one against its time window.
pub fn validate_speech_rate(script: &NarrationScript, lang: &str) -> Vec<SegmentOverflow> {
    script
        .segments
        .iter()
        .map(|seg| {
            let window = (seg.end_seconds - seg.start_seconds).max(0.001);
            let predicted = estimate_tts_seconds(&seg.text, lang);
            SegmentOverflow::from_ratio(seg.index, predicted, window)
        })
        .collect()
}

/// Fraction of segments (0.0..=1.0) classified as `Severity::Overflow`. Used
/// to decide whether to re-prompt the LLM with the budget re-stated.
pub fn overflow_fraction(report: &[SegmentOverflow]) -> f64 {
    if report.is_empty() {
        return 0.0;
    }
    let overflows = report
        .iter()
        .filter(|o| o.severity == Severity::Overflow)
        .count();
    overflows as f64 / report.len() as f64
}

/// Render the overflow report as a human/LLM-readable string that can be
/// appended to the user message on retry. Lists only segments that exceed
/// their budget.
pub fn format_retry_feedback(report: &[SegmentOverflow], lang: &str) -> String {
    let mut out = String::new();
    out.push_str(
        "Some segments exceeded their word budget in the previous draft. \
         Rewrite the script so every segment fits within its time window at \
         natural speech pace. Specifically shorten these:\n",
    );
    for o in report.iter().filter(|o| o.severity == Severity::Overflow) {
        out.push_str(&format!(
            "  - segment {}: predicted ~{:.1}s of speech in a {:.1}s window (over by {:.1}s)\n",
            o.index,
            o.predicted_seconds,
            o.window_seconds,
            o.predicted_seconds - o.window_seconds,
        ));
    }
    out.push_str(&format!(
        "\nReminder: target speech rate is {:.0} {} per minute for language '{}'. \
         Either shorten the text or widen the segment's time window.\n",
        crate::speech_rate::rate_per_minute(lang),
        crate::speech_rate::budget_unit(lang),
        lang,
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{NarrationScript, Pace, ScriptMetadata, Segment};

    fn mk_script(segments: Vec<(f64, f64, &str)>) -> NarrationScript {
        NarrationScript {
            title: "t".into(),
            total_duration_seconds: segments.last().map(|s| s.1).unwrap_or(0.0),
            segments: segments
                .into_iter()
                .enumerate()
                .map(|(i, (s, e, text))| Segment {
                    index: i,
                    start_seconds: s,
                    end_seconds: e,
                    text: text.into(),
                    visual_description: String::new(),
                    emphasis: vec![],
                    pace: Pace::Medium,
                    pause_after_ms: 0,
                    frame_refs: vec![],
                    voice_override: None,
                })
                .collect(),
            metadata: ScriptMetadata::default(),
            speech_rate_report: None,
        }
    }

    #[test]
    fn flags_overflow_segment() {
        // 10s window, ~50 words → ~20s of speech at 150 wpm → ratio 2.0 → Overflow.
        let text = "word ".repeat(50);
        let script = mk_script(vec![(0.0, 10.0, text.trim())]);
        let report = validate_speech_rate(&script, "en");
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].severity, Severity::Overflow);
        assert!((report[0].predicted_seconds - 20.0).abs() < 1e-6);
    }

    #[test]
    fn marks_tight_segment() {
        // 24 words in a 10s window at 150 wpm → 9.6s → ratio 0.96 → Tight.
        let text = "word ".repeat(24);
        let script = mk_script(vec![(0.0, 10.0, text.trim())]);
        let report = validate_speech_rate(&script, "en");
        assert_eq!(report[0].severity, Severity::Tight);
    }

    #[test]
    fn marks_compress_segment() {
        // 28 words in a 10s window at 150 wpm → 11.2s → ratio 1.12 → Compress.
        let text = "word ".repeat(28);
        let script = mk_script(vec![(0.0, 10.0, text.trim())]);
        let report = validate_speech_rate(&script, "en");
        assert_eq!(report[0].severity, Severity::Compress);
    }

    #[test]
    fn passes_clean_script() {
        // Plenty of breathing room.
        let text = "word ".repeat(10);
        let script = mk_script(vec![(0.0, 10.0, text.trim())]);
        let report = validate_speech_rate(&script, "en");
        assert_eq!(report[0].severity, Severity::Fit);
    }

    #[test]
    fn overflow_fraction_counts_only_overflows() {
        let big = "word ".repeat(100);
        let small = "word".to_string();
        let script = mk_script(vec![
            (0.0, 10.0, big.trim()),
            (10.0, 20.0, &small),
            (20.0, 30.0, big.trim()),
        ]);
        let report = validate_speech_rate(&script, "en");
        assert!((overflow_fraction(&report) - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn retry_feedback_lists_overflow_indices() {
        let big = "word ".repeat(100);
        let script = mk_script(vec![(0.0, 10.0, big.trim()), (10.0, 20.0, "hi")]);
        let report = validate_speech_rate(&script, "en");
        let msg = format_retry_feedback(&report, "en");
        assert!(msg.contains("segment 0"));
        assert!(!msg.contains("segment 1"));
        assert!(msg.contains("words per minute") || msg.contains("150"));
    }

    #[test]
    fn zero_width_segment_does_not_panic() {
        let script = mk_script(vec![(5.0, 5.0, "hi")]);
        let report = validate_speech_rate(&script, "en");
        // The segment has zero time but any nonzero predicted → Overflow.
        assert_eq!(report[0].severity, Severity::Overflow);
    }
}
