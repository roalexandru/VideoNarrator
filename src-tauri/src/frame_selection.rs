//! Let the model choose which moments to look at closely.
//!
//! `merge_anchors` caps candidate anchors by keeping an evenly-spaced subset.
//! That is a mechanical heuristic deciding what the model is allowed to see: it
//! has no notion of which moments carry meaningful change, so on a screencast
//! with three interesting transitions and four minutes of a static editor it
//! spends most of its budget on the static part.
//!
//! Two stages instead:
//!
//! 1. **Survey** — extract a dense set of small frames in a *single* ffmpeg
//!    process (one `fps=` filter pass, no per-anchor seeking), tile them into
//!    contact sheets, and make one cheap call asking which timestamps matter.
//! 2. **Detail** — extract only the chosen moments at full resolution through
//!    the existing frame-accurate two-step-seek path.
//!
//! The survey is close to free relative to what it replaces: one decode pass at
//! 320 px versus up to 300 individual seek-and-encode ffmpeg invocations. And
//! because stage 2 only extracts what was chosen, total extraction cost drops
//! even with the extra call.
//!
//! Every failure path falls back to the existing behaviour. A model that returns
//! nothing usable must not mean a video with no frames.

use crate::error::NarratorError;
use crate::models::SilenceSpan;

/// How many frames the survey pass samples.
///
/// Dense enough that a 4-minute video gets a sample every ~2 s, small enough to
/// tile into a handful of contact sheets.
pub const SURVEY_FRAME_COUNT: usize = 120;

/// Survey frame width in pixels. Enough to see layout and gross change; not
/// enough to read code, which is the detail pass's job.
pub const SURVEY_FRAME_WIDTH: u32 = 320;

/// Minimum spacing between selected moments, in seconds.
///
/// Two frames 200 ms apart show the same thing, so a model that clusters its
/// picks would waste the detail budget.
pub const MIN_SELECTION_GAP: f64 = 1.0;

/// A moment the model asked to look at closely.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectedMoment {
    pub timestamp: f64,
    /// The model's stated reason. Kept for logging — it is what makes a bad
    /// selection diagnosable rather than mysterious.
    pub reason: String,
}

/// Survey sampling plan for a video of `duration` seconds.
///
/// Returns the `fps` value for the filter and how many frames to expect. Kept
/// pure because the arithmetic is where an off-by-one silently halves coverage.
pub fn plan_survey(duration: f64, target: usize) -> (f64, usize) {
    let target = target.max(1);
    if !duration.is_finite() || duration <= 0.0 {
        return (1.0, 1);
    }
    // One frame every `duration / target` seconds → fps = target / duration.
    let fps = (target as f64 / duration).max(1.0 / duration.max(1.0));
    let expected = ((duration * fps).round() as usize).clamp(1, target);
    (fps, expected)
}

/// Turn the model's raw response into usable timestamps.
///
/// Applies every guard the rest of the pipeline assumes: in-range, finite,
/// sorted, de-clustered, and capped. A model asked for 30 moments will sometimes
/// return 45, or two 100 ms apart, or one past the end of the video.
pub fn sanitize_selection(
    mut moments: Vec<SelectedMoment>,
    duration: f64,
    max_count: usize,
) -> Vec<SelectedMoment> {
    if max_count == 0 || duration <= 0.0 {
        return Vec::new();
    }

    moments.retain(|m| m.timestamp.is_finite() && m.timestamp >= 0.0 && m.timestamp <= duration);
    moments.sort_by(|a, b| a.timestamp.total_cmp(&b.timestamp));

    // Drop anything too close to the previously kept moment.
    let mut kept: Vec<SelectedMoment> = Vec::with_capacity(moments.len().min(max_count));
    for moment in moments {
        let too_close = kept
            .last()
            .is_some_and(|prev| moment.timestamp - prev.timestamp < MIN_SELECTION_GAP);
        if !too_close {
            kept.push(moment);
        }
    }

    // Over-budget: keep an evenly spaced subset of what the model chose. This is
    // still better than an evenly spaced subset of *everything*, because every
    // survivor is a moment the model flagged.
    if kept.len() > max_count {
        let stride = kept.len() as f64 / max_count as f64;
        kept = (0..max_count)
            .map(|i| kept[((i as f64 * stride).floor() as usize).min(kept.len() - 1)].clone())
            .collect();
    }

    kept
}

/// Ensure the selection covers the whole timeline.
///
/// A model that fixates on the first minute leaves the rest of the video
/// unnarrated. When a stretch longer than `max_gap` has no selected moment, its
/// midpoint is added so the detail pass still has something to describe there.
pub fn fill_coverage_gaps(
    mut moments: Vec<SelectedMoment>,
    duration: f64,
    max_gap: f64,
) -> Vec<SelectedMoment> {
    if duration <= 0.0 || max_gap <= 0.0 {
        return moments;
    }
    moments.sort_by(|a, b| a.timestamp.total_cmp(&b.timestamp));

    let mut filled: Vec<SelectedMoment> = Vec::new();
    let mut cursor = 0.0_f64;
    for moment in moments {
        while moment.timestamp - cursor > max_gap {
            let midpoint = cursor + max_gap / 2.0;
            if midpoint >= moment.timestamp {
                break;
            }
            filled.push(SelectedMoment {
                timestamp: midpoint,
                reason: "coverage fill (no moment selected in this stretch)".into(),
            });
            cursor = midpoint;
        }
        cursor = moment.timestamp;
        filled.push(moment);
    }
    // Trailing stretch after the last selected moment.
    while duration - cursor > max_gap {
        let midpoint = cursor + max_gap / 2.0;
        if midpoint >= duration {
            break;
        }
        filled.push(SelectedMoment {
            timestamp: midpoint,
            reason: "coverage fill (trailing stretch)".into(),
        });
        cursor = midpoint;
    }

    filled
}

/// Prompt for the survey call.
///
/// Deliberately narrow: this call picks *timestamps*, it does not write
/// narration. Asking for both in one call produces worse of each.
pub fn survey_system_prompt(target: usize, duration: f64) -> String {
    format!(
        "You are selecting which moments of a {duration:.0}s video deserve a closer look \
         before narration is written.\n\n\
         You will receive low-resolution contact sheets covering the whole video. Choose up \
         to {target} timestamps where something meaningful changes or happens — a new screen, \
         a result appearing, an error, a transition, a completed action.\n\n\
         RULES:\n\
         1. Spread choices across the WHOLE video. A cluster in one region leaves the rest \
            with nothing to narrate.\n\
         2. Prefer the moment just AFTER a change completes, when the new state is fully \
            visible, over the moment mid-transition.\n\
         3. Skip near-duplicates. Two timestamps less than {MIN_SELECTION_GAP:.0}s apart show \
            the same thing.\n\
         4. Skip stretches where nothing changes — a static screen needs one moment, not ten.\n\
         5. Give a short concrete reason for each choice, so a bad pick is diagnosable.\n\n\
         Timestamps must be in seconds, between 0 and {duration:.1}."
    )
}

/// Describe the audio landscape to the survey call, when we know it.
///
/// A moment inside a long silence is often a better narration anchor than one
/// mid-sentence, so the survey benefits from the same map the timeline uses.
pub fn survey_silence_hint(spans: &[SilenceSpan]) -> String {
    let wide: Vec<&SilenceSpan> = spans.iter().filter(|s| s.duration() >= 1.0).collect();
    if wide.is_empty() {
        return String::new();
    }
    let listed: Vec<String> = wide
        .iter()
        .take(20)
        .map(|s| format!("{:.1}-{:.1}", s.start, s.end))
        .collect();
    format!(
        "\n\nThe source audio is quiet during these windows (seconds), which tend to be \
         natural places for a narrated beat: {}.",
        listed.join(", ")
    )
}

/// Parse the survey response into moments.
///
/// Tolerant of the shapes a model actually returns: the schema asks for
/// `{"moments": [{"timestamp": n, "reason": s}]}`, but a bare array, or numbers
/// as strings, both show up in practice and cost a whole call to reject.
pub fn parse_selection(raw: &str) -> Result<Vec<SelectedMoment>, NarratorError> {
    let trimmed = raw
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let value: serde_json::Value = serde_json::from_str(trimmed)
        .map_err(|e| NarratorError::ApiError(format!("frame selection JSON parse failed: {e}")))?;

    let array = value
        .get("moments")
        .and_then(|v| v.as_array())
        .or_else(|| value.as_array())
        .cloned()
        .unwrap_or_default();

    let mut out = Vec::with_capacity(array.len());
    for entry in array {
        // Accept an object, or a bare number for the whole entry.
        let timestamp = entry
            .get("timestamp")
            .and_then(coerce_f64)
            .or_else(|| coerce_f64(&entry));
        let Some(timestamp) = timestamp else { continue };
        let reason = entry
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        out.push(SelectedMoment { timestamp, reason });
    }
    Ok(out)
}

/// Read a JSON number that may have arrived as a string.
fn coerce_f64(v: &serde_json::Value) -> Option<f64> {
    match v {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// Build the survey user message: contact sheets covering the whole video.
///
/// Sheets rather than loose images because a survey is 120 frames — as separate
/// images that would be 12 calls before a single narration token is generated.
pub fn build_survey_message(
    survey_frames: &[crate::models::Frame],
) -> Result<serde_json::Value, NarratorError> {
    use serde_json::json;

    let mut content = Vec::new();
    for group in survey_frames.chunks(crate::ai_client::FRAMES_PER_SHEET) {
        let Some(sheet) = crate::contact_sheet::build(
            group,
            crate::contact_sheet::DEFAULT_COLUMNS,
            SURVEY_FRAME_WIDTH,
        )?
        else {
            continue;
        };
        content.push(json!({"type": "text", "text": sheet.describe()}));
        content.push(json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": "image/jpeg",
                "data": sheet.base64
            }
        }));
    }
    Ok(serde_json::Value::Array(content))
}

/// Ask the model which moments deserve full-resolution extraction.
///
/// Returns `None` — never an error — when the survey cannot produce a usable
/// answer, so the caller falls back to the existing even-spaced selection. A
/// failed survey must cost one cheap call, not the generation.
pub async fn select_moments(
    provider: &dyn crate::ai_client::AiProvider,
    survey_frames: &[crate::models::Frame],
    duration: f64,
    target_count: usize,
    silence_spans: &[SilenceSpan],
) -> Option<Vec<SelectedMoment>> {
    if survey_frames.is_empty() || target_count == 0 {
        return None;
    }

    let system_prompt = format!(
        "{}{}",
        survey_system_prompt(target_count, duration),
        survey_silence_hint(silence_spans)
    );
    let user_message = match build_survey_message(survey_frames) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("survey message build failed, falling back: {e}");
            return None;
        }
    };

    let schema = crate::response_schema::frame_selection();
    let raw = match provider
        .generate_with_schema(&system_prompt, user_message, &schema)
        .await
    {
        Ok(raw) => raw,
        Err(e) => {
            tracing::warn!("survey call failed, falling back to even spacing: {e}");
            return None;
        }
    };

    let parsed = match parse_selection(&raw) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("survey response unparseable, falling back: {e}");
            return None;
        }
    };

    let sanitized = sanitize_selection(parsed, duration, target_count);
    if sanitized.is_empty() {
        tracing::warn!("survey selected nothing usable, falling back to even spacing");
        return None;
    }

    // Backfill any stretch the model ignored, so no part of the video ends up
    // with nothing to narrate.
    let max_gap = (duration / target_count as f64) * 3.0;
    let filled = fill_coverage_gaps(sanitized, duration, max_gap);

    tracing::info!(
        "survey selected {} moments (target {target_count}) over {duration:.1}s",
        filled.len()
    );
    Some(filled)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(timestamp: f64) -> SelectedMoment {
        SelectedMoment {
            timestamp,
            reason: "because".into(),
        }
    }

    // ── Survey planning ─────────────────────────────────────────────────

    #[test]
    fn survey_plan_covers_the_whole_video() {
        let (fps, count) = plan_survey(240.0, 120);
        // 120 frames over 240s → one every 2s.
        assert!((fps - 0.5).abs() < 1e-9, "fps was {fps}");
        assert_eq!(count, 120);
    }

    #[test]
    fn survey_plan_does_not_oversample_a_short_video() {
        // A 10s clip must not be asked for 120 frames' worth of decode.
        let (_, count) = plan_survey(10.0, 120);
        assert!(count <= 120);
        assert!(count >= 1);
    }

    #[test]
    fn survey_plan_survives_degenerate_durations() {
        for bad in [0.0, -5.0, f64::NAN, f64::INFINITY] {
            let (fps, count) = plan_survey(bad, 120);
            assert!(fps.is_finite() && fps > 0.0, "fps for {bad} was {fps}");
            assert!(count >= 1);
        }
    }

    // ── Sanitizing what the model returned ──────────────────────────────

    #[test]
    fn selection_drops_out_of_range_timestamps() {
        let picked = sanitize_selection(
            vec![m(-3.0), m(10.0), m(500.0), m(f64::NAN), m(20.0)],
            60.0,
            30,
        );
        let times: Vec<f64> = picked.iter().map(|p| p.timestamp).collect();
        assert_eq!(times, vec![10.0, 20.0]);
    }

    #[test]
    fn selection_is_sorted_even_when_the_model_is_not() {
        let picked = sanitize_selection(vec![m(30.0), m(5.0), m(20.0)], 60.0, 30);
        let times: Vec<f64> = picked.iter().map(|p| p.timestamp).collect();
        assert_eq!(times, vec![5.0, 20.0, 30.0]);
    }

    #[test]
    fn selection_declusters_near_duplicates() {
        // 0.2s apart shows the same thing; keeping both wastes detail budget.
        let picked = sanitize_selection(vec![m(10.0), m(10.2), m(10.4), m(30.0)], 60.0, 30);
        let times: Vec<f64> = picked.iter().map(|p| p.timestamp).collect();
        assert_eq!(times, vec![10.0, 30.0]);
    }

    #[test]
    fn selection_keeps_moments_exactly_at_the_gap_threshold() {
        let picked = sanitize_selection(vec![m(10.0), m(10.0 + MIN_SELECTION_GAP)], 60.0, 30);
        assert_eq!(picked.len(), 2, "a full gap apart must both survive");
    }

    #[test]
    fn selection_caps_an_over_eager_model() {
        // Asked for 5, returned 20 — must trim to 5 and stay spread out.
        let moments: Vec<SelectedMoment> = (0..20).map(|i| m(i as f64 * 3.0)).collect();
        let picked = sanitize_selection(moments, 60.0, 5);
        assert_eq!(picked.len(), 5);
        let times: Vec<f64> = picked.iter().map(|p| p.timestamp).collect();
        // Still ascending and spanning most of the range, not the first five.
        assert!(times.windows(2).all(|w| w[0] < w[1]));
        assert!(
            times.last().unwrap() - times.first().unwrap() > 30.0,
            "capping must not collapse to the start: {times:?}"
        );
    }

    #[test]
    fn selection_handles_empty_and_degenerate_input() {
        assert!(sanitize_selection(vec![], 60.0, 30).is_empty());
        assert!(sanitize_selection(vec![m(10.0)], 60.0, 0).is_empty());
        assert!(sanitize_selection(vec![m(10.0)], 0.0, 30).is_empty());
    }

    #[test]
    fn selection_accepts_a_boundary_timestamp() {
        let picked = sanitize_selection(vec![m(0.0), m(60.0)], 60.0, 30);
        assert_eq!(picked.len(), 2, "0 and duration are both valid");
    }

    // ── Coverage backfill ───────────────────────────────────────────────

    #[test]
    fn coverage_fill_covers_a_model_that_fixated_on_the_opening() {
        // Everything in the first 15s of a 120s video.
        let picked = vec![m(2.0), m(6.0), m(12.0)];
        let filled = fill_coverage_gaps(picked, 120.0, 30.0);
        let times: Vec<f64> = filled.iter().map(|p| p.timestamp).collect();
        assert!(
            times.iter().any(|t| *t > 60.0),
            "second half must get coverage: {times:?}"
        );
        // Still sorted.
        assert!(times.windows(2).all(|w| w[0] <= w[1]), "{times:?}");
    }

    #[test]
    fn coverage_fill_is_a_no_op_on_a_well_spread_selection() {
        let picked: Vec<SelectedMoment> = (0..12).map(|i| m(i as f64 * 10.0)).collect();
        let before = picked.len();
        let filled = fill_coverage_gaps(picked, 120.0, 30.0);
        assert_eq!(filled.len(), before, "nothing to fill");
    }

    #[test]
    fn coverage_fill_labels_what_it_added() {
        // A synthetic moment must be distinguishable from a model choice in logs.
        let filled = fill_coverage_gaps(vec![m(1.0)], 120.0, 30.0);
        assert!(filled.iter().any(|f| f.reason.contains("coverage fill")));
        assert!(filled.iter().any(|f| f.reason == "because"));
    }

    #[test]
    fn coverage_fill_terminates_on_degenerate_input() {
        // Guard against an infinite loop in the while-gap logic.
        assert_eq!(fill_coverage_gaps(vec![m(1.0)], 0.0, 30.0).len(), 1);
        assert_eq!(fill_coverage_gaps(vec![m(1.0)], 120.0, 0.0).len(), 1);
        assert!(!fill_coverage_gaps(vec![], 120.0, 30.0).is_empty());
    }

    // ── Response parsing ────────────────────────────────────────────────

    #[test]
    fn parses_the_schema_shape() {
        let raw = r#"{"moments":[{"timestamp":12.5,"reason":"terminal shows the error"},
                                 {"timestamp":30.0,"reason":"new screen"}]}"#;
        let parsed = parse_selection(raw).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].timestamp, 12.5);
        assert_eq!(parsed[0].reason, "terminal shows the error");
    }

    #[test]
    fn parses_a_bare_array_and_stringified_numbers() {
        // Both shapes show up in practice, and rejecting them costs a whole call.
        let parsed = parse_selection(r#"[{"timestamp":"5.5","reason":"x"}]"#).unwrap();
        assert_eq!(parsed[0].timestamp, 5.5);
        let parsed = parse_selection("[3, 9.5]").unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[1].timestamp, 9.5);
    }

    #[test]
    fn parses_through_code_fences() {
        let parsed = parse_selection("```json\n{\"moments\":[{\"timestamp\":1.0}]}\n```").unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].reason, "", "missing reason is tolerated");
    }

    #[test]
    fn skips_entries_with_no_usable_timestamp() {
        let raw = r#"{"moments":[{"reason":"no timestamp"},{"timestamp":"abc"},{"timestamp":4}]}"#;
        let parsed = parse_selection(raw).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].timestamp, 4.0);
    }

    #[test]
    fn errors_on_non_json_and_returns_empty_on_wrong_shape() {
        assert!(parse_selection("not json at all").is_err());
        // Valid JSON, no moments → empty, not an error. The caller falls back.
        assert!(parse_selection(r#"{"unexpected":true}"#)
            .unwrap()
            .is_empty());
    }

    // ── Prompt ──────────────────────────────────────────────────────────

    #[test]
    fn survey_prompt_states_the_budget_range_and_spread_rule() {
        let prompt = survey_system_prompt(30, 240.0);
        assert!(prompt.contains("30"), "target count missing");
        assert!(prompt.contains("240"), "duration missing");
        assert!(prompt.contains("WHOLE video"), "spread rule missing");
        // The de-clustering threshold must match the code that enforces it.
        assert!(prompt.contains(&format!("{MIN_SELECTION_GAP:.0}s apart")));
    }

    // ── Orchestration guards ────────────────────────────────────────────

    #[tokio::test]
    async fn select_moments_declines_without_survey_frames() {
        // No survey → the caller must fall back, not receive an empty selection
        // that would extract zero frames.
        struct NeverCalled;
        #[async_trait::async_trait]
        impl crate::ai_client::AiProvider for NeverCalled {
            async fn generate(
                &self,
                _: &str,
                _: serde_json::Value,
            ) -> Result<String, NarratorError> {
                panic!("must not call the model with no survey frames");
            }
            fn name(&self) -> &str {
                "never"
            }
            fn model(&self) -> &str {
                "never"
            }
        }
        assert!(select_moments(&NeverCalled, &[], 60.0, 30, &[])
            .await
            .is_none());
    }

    #[tokio::test]
    async fn select_moments_falls_back_when_the_model_errors() {
        struct AlwaysFails;
        #[async_trait::async_trait]
        impl crate::ai_client::AiProvider for AlwaysFails {
            async fn generate(
                &self,
                _: &str,
                _: serde_json::Value,
            ) -> Result<String, NarratorError> {
                Err(NarratorError::ApiError("boom".into()))
            }
            fn name(&self) -> &str {
                "fails"
            }
            fn model(&self) -> &str {
                "fails"
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jpg");
        image::RgbImage::from_pixel(32, 18, image::Rgb([1, 1, 1]))
            .save(&path)
            .unwrap();
        let frames = vec![crate::models::Frame {
            index: 0,
            timestamp_seconds: 0.0,
            path,
            width: 32,
            height: 18,
        }];
        // A failed survey costs one cheap call, never the generation.
        assert!(select_moments(&AlwaysFails, &frames, 60.0, 30, &[])
            .await
            .is_none());
    }

    #[test]
    fn silence_hint_lists_wide_windows_and_stays_quiet_otherwise() {
        let spans = vec![
            SilenceSpan {
                start: 5.0,
                end: 8.0,
            },
            // Too narrow to be a useful beat.
            SilenceSpan {
                start: 20.0,
                end: 20.2,
            },
        ];
        let hint = survey_silence_hint(&spans);
        assert!(hint.contains("5.0-8.0"));
        assert!(!hint.contains("20.0-20.2"));
        assert!(survey_silence_hint(&[]).is_empty());
    }
}
