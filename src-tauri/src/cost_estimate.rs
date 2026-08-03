//! What a generation is about to cost, before it starts.
//!
//! Generation begins spending immediately: frames are encoded and sent, and on a
//! long video that is thirty API calls carrying three hundred images. A user who
//! left the density on Heavy for a forty-minute screencast finds out afterwards.
//!
//! This turns the settings into a forecast — request count, image count, rough
//! input tokens — so the Processing screen can show it and ask before spending.
//!
//! ## On accuracy
//!
//! These are estimates and are labelled as such. Image token cost is
//! resolution-dependent and differs per provider; prompt overhead varies with
//! style and context documents. The number that matters is the order of
//! magnitude: "4 requests, ~40k tokens" versus "30 requests, ~400k tokens" is
//! the decision the user is making, and that distinction is robust even if the
//! per-image constant is off by a third.

use crate::models::FrameConfig;
use serde::{Deserialize, Serialize};

/// Approximate input tokens for one 1024 px-wide frame.
///
/// Anthropic's guidance is roughly `(width × height) / 750`; a 1024×576 frame
/// lands near 780. Rounded up, and used for all providers — they are within a
/// factor of ~1.5 of each other, which is inside this estimate's stated
/// precision.
pub const TOKENS_PER_FULL_FRAME: usize = 800;

/// Approximate input tokens for one contact sheet.
///
/// A 3x3 sheet of 512 px cells is ~1536 px wide, so it costs more than one frame
/// but far less than the nine it replaces — which is the entire point of tiling.
pub const TOKENS_PER_CONTACT_SHEET: usize = 1_800;

/// Rough token cost of the system prompt plus per-request text scaffolding.
pub const TOKENS_PROMPT_OVERHEAD: usize = 1_500;

/// Approximate output tokens per narration segment.
pub const TOKENS_PER_SEGMENT_OUT: usize = 120;

/// Seconds of narration a segment typically covers. Used to guess segment count.
const SECONDS_PER_SEGMENT: f64 = 8.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerationEstimate {
    /// Frames that will be extracted and sent.
    pub frame_count: usize,
    /// Images actually sent — equals `frame_count` untiled, or the number of
    /// contact sheets when tiling.
    pub image_count: usize,
    /// API calls the narration pass will make.
    pub request_count: usize,
    /// Rough input tokens across all requests.
    pub input_tokens: usize,
    /// Rough output tokens.
    pub output_tokens: usize,
    /// True when a survey pass adds a request before narration starts.
    pub includes_survey: bool,
    /// True when previously extracted frames will be reused.
    pub reuses_cached_frames: bool,
    /// One-line summary for display, built by [`describe`].
    ///
    /// Formatted here rather than in the frontend so the phrasing (and the
    /// pluralisation) stays testable.
    pub summary: String,
}

/// Inputs the estimate depends on.
#[derive(Debug, Clone, Copy)]
pub struct EstimateInputs {
    pub duration_seconds: f64,
    /// Images allowed per request — `ai_client::MAX_FRAMES_PER_CALL`.
    pub images_per_request: usize,
    /// Images per contact sheet when tiling.
    pub frames_per_sheet: usize,
    pub tiled: bool,
    pub model_selection: bool,
    /// Frames the survey pass samples, when `model_selection` is on. Passed in
    /// rather than read from `frame_selection` so this module stays a pure
    /// function of its inputs.
    pub survey_frame_count: usize,
    pub strict_mode: bool,
    pub cached_frames: bool,
}

/// Frames a config will actually produce for a video of `duration` seconds.
///
/// Mirrors the adaptive widening in `extract_frames_fixed_interval`: the interval
/// stretches when the naive count would exceed `max_frames`, so the estimate must
/// not simply report the cap.
pub fn expected_frame_count(duration: f64, config: &FrameConfig) -> usize {
    if duration <= 0.0 || !duration.is_finite() {
        return 0;
    }
    let interval = config.density.interval_seconds().max(0.001);
    let naive = (duration / interval).ceil() as usize;
    naive.clamp(1, config.max_frames.max(1))
}

/// Forecast a generation.
pub fn estimate(config: &FrameConfig, inputs: EstimateInputs) -> GenerationEstimate {
    let frame_count = expected_frame_count(inputs.duration_seconds, config);

    let image_count = if inputs.tiled && inputs.frames_per_sheet > 0 {
        frame_count.div_ceil(inputs.frames_per_sheet)
    } else {
        frame_count
    };

    let per_request = inputs.images_per_request.max(1);
    // At least one request even for a single image.
    let mut request_count = image_count.div_ceil(per_request).max(1);

    let tokens_per_image = if inputs.tiled {
        TOKENS_PER_CONTACT_SHEET
    } else {
        TOKENS_PER_FULL_FRAME
    };
    let mut input_tokens = image_count * tokens_per_image + request_count * TOKENS_PROMPT_OVERHEAD;

    // The survey is one extra request carrying its own (cheap) sheets.
    if inputs.model_selection {
        let survey_sheets = inputs
            .survey_frame_count
            .div_ceil(inputs.frames_per_sheet.max(1));
        request_count += 1;
        input_tokens += survey_sheets * TOKENS_PER_CONTACT_SHEET + TOKENS_PROMPT_OVERHEAD;
    }

    let segment_count = (inputs.duration_seconds / SECONDS_PER_SEGMENT)
        .ceil()
        .max(1.0) as usize;
    let mut output_tokens = segment_count * TOKENS_PER_SEGMENT_OUT;

    // Strict mode adds a critique call plus up to a handful of refine calls, and
    // the critique re-sends frames.
    if inputs.strict_mode {
        request_count += 1 + 5;
        input_tokens += 10 * TOKENS_PER_FULL_FRAME + 6 * TOKENS_PROMPT_OVERHEAD;
        output_tokens += 5 * TOKENS_PER_SEGMENT_OUT;
    }

    let mut estimate = GenerationEstimate {
        frame_count,
        image_count,
        request_count,
        input_tokens,
        output_tokens,
        includes_survey: inputs.model_selection,
        reuses_cached_frames: inputs.cached_frames,
        summary: String::new(),
    };
    estimate.summary = describe(&estimate);
    estimate
}

/// Human-readable summary for the pre-flight panel.
pub fn describe(estimate: &GenerationEstimate) -> String {
    let mut parts = vec![format!(
        "{} frames in {} request{}, roughly {}k input tokens",
        estimate.frame_count,
        estimate.request_count,
        if estimate.request_count == 1 { "" } else { "s" },
        (estimate.input_tokens as f64 / 1000.0).round() as usize
    )];
    if estimate.image_count != estimate.frame_count {
        parts.push(format!("{} tiled images", estimate.image_count));
    }
    if estimate.includes_survey {
        parts.push("includes a survey pass".into());
    }
    if estimate.reuses_cached_frames {
        parts.push("frames already extracted".into());
    }
    parts.join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{FrameDensity, RenderQuality};

    fn config(density: FrameDensity, max_frames: usize) -> FrameConfig {
        FrameConfig {
            density,
            scene_threshold: 0.3,
            max_frames,
            skip_dedup: true,
        }
    }

    fn inputs(duration: f64) -> EstimateInputs {
        EstimateInputs {
            duration_seconds: duration,
            images_per_request: 10,
            frames_per_sheet: 9,
            tiled: false,
            model_selection: false,
            survey_frame_count: 120,
            strict_mode: false,
            cached_frames: false,
        }
    }

    // ── Frame count ─────────────────────────────────────────────────────

    #[test]
    fn frame_count_follows_density() {
        // 120s at medium (5s interval) → 24 frames.
        assert_eq!(
            expected_frame_count(120.0, &config(FrameDensity::Medium, 300)),
            24
        );
        // Heavy is 2s → 60.
        assert_eq!(
            expected_frame_count(120.0, &config(FrameDensity::Heavy, 300)),
            60
        );
        // Light is 10s → 12.
        assert_eq!(
            expected_frame_count(120.0, &config(FrameDensity::Light, 300)),
            12
        );
    }

    #[test]
    fn frame_count_respects_the_cap_rather_than_reporting_it_blindly() {
        // A 40-minute video at heavy would want 1200 frames but is capped.
        assert_eq!(
            expected_frame_count(2400.0, &config(FrameDensity::Heavy, 300)),
            300
        );
        // A short video must report its real (small) count, not the cap — the
        // whole point is telling the user what will actually happen.
        assert_eq!(
            expected_frame_count(30.0, &config(FrameDensity::Medium, 300)),
            6
        );
    }

    #[test]
    fn frame_count_handles_degenerate_durations() {
        for bad in [0.0, -10.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                expected_frame_count(bad, &config(FrameDensity::Medium, 300)),
                0
            );
        }
    }

    // ── Requests ────────────────────────────────────────────────────────

    #[test]
    fn requests_scale_with_images_not_frames_when_tiling() {
        // This is the ratio the tiling feature exists to change.
        let cfg = config(FrameDensity::Heavy, 300);
        let untiled = estimate(&cfg, inputs(600.0));
        let tiled = estimate(
            &cfg,
            EstimateInputs {
                tiled: true,
                ..inputs(600.0)
            },
        );
        assert_eq!(untiled.frame_count, tiled.frame_count, "same frames sent");
        assert!(
            tiled.request_count < untiled.request_count,
            "tiling must reduce requests: {} vs {}",
            tiled.request_count,
            untiled.request_count
        );
        assert!(tiled.image_count < untiled.image_count);
    }

    #[test]
    fn a_short_video_still_costs_one_request() {
        let est = estimate(&config(FrameDensity::Light, 300), inputs(5.0));
        assert_eq!(est.request_count, 1);
        assert!(est.input_tokens > 0);
    }

    #[test]
    fn thirty_requests_is_what_a_long_heavy_job_actually_costs() {
        // The scenario the pre-flight panel exists to warn about.
        let est = estimate(&config(FrameDensity::Heavy, 300), inputs(2400.0));
        assert_eq!(est.frame_count, 300);
        assert_eq!(est.request_count, 30);
        assert!(
            est.input_tokens > 200_000,
            "expected a large token estimate, got {}",
            est.input_tokens
        );
    }

    #[test]
    fn survey_adds_exactly_one_request() {
        let cfg = config(FrameDensity::Medium, 300);
        let without = estimate(&cfg, inputs(120.0));
        let with = estimate(
            &cfg,
            EstimateInputs {
                model_selection: true,
                ..inputs(120.0)
            },
        );
        assert_eq!(with.request_count, without.request_count + 1);
        assert!(with.includes_survey);
        assert!(
            with.input_tokens > without.input_tokens,
            "survey costs tokens"
        );
    }

    #[test]
    fn strict_mode_is_reflected_in_the_estimate() {
        // Strict mode roughly doubles small jobs; the user should see that.
        let cfg = config(FrameDensity::Medium, 300);
        let plain = estimate(&cfg, inputs(120.0));
        let strict = estimate(
            &cfg,
            EstimateInputs {
                strict_mode: true,
                ..inputs(120.0)
            },
        );
        assert!(strict.request_count > plain.request_count);
        assert!(strict.output_tokens > plain.output_tokens);
    }

    #[test]
    fn output_tokens_scale_with_duration() {
        let cfg = config(FrameDensity::Medium, 300);
        let short = estimate(&cfg, inputs(60.0));
        let long = estimate(&cfg, inputs(600.0));
        assert!(long.output_tokens > short.output_tokens);
    }

    #[test]
    fn cached_frames_are_reported_without_changing_token_cost() {
        // Caching saves extraction time, not tokens — the frames are still sent.
        let cfg = config(FrameDensity::Medium, 300);
        let fresh = estimate(&cfg, inputs(120.0));
        let cached = estimate(
            &cfg,
            EstimateInputs {
                cached_frames: true,
                ..inputs(120.0)
            },
        );
        assert!(cached.reuses_cached_frames);
        assert_eq!(cached.input_tokens, fresh.input_tokens);
    }

    // ── Description ─────────────────────────────────────────────────────

    #[test]
    fn description_states_frames_requests_and_tokens() {
        let est = estimate(&config(FrameDensity::Heavy, 300), inputs(2400.0));
        let text = describe(&est);
        assert!(text.contains("300 frames"), "{text}");
        assert!(text.contains("30 requests"), "{text}");
        assert!(text.contains("input tokens"), "{text}");
    }

    #[test]
    fn description_pluralizes_a_single_request() {
        let est = estimate(&config(FrameDensity::Light, 300), inputs(5.0));
        let text = describe(&est);
        assert!(text.contains("1 request,"), "{text}");
        assert!(!text.contains("1 requests"), "{text}");
    }

    #[test]
    fn description_mentions_tiling_and_survey_only_when_they_apply() {
        let cfg = config(FrameDensity::Medium, 300);
        let plain = describe(&estimate(&cfg, inputs(120.0)));
        assert!(!plain.contains("tiled"));
        assert!(!plain.contains("survey"));

        let fancy = describe(&estimate(
            &cfg,
            EstimateInputs {
                tiled: true,
                model_selection: true,
                cached_frames: true,
                ..inputs(120.0)
            },
        ));
        assert!(fancy.contains("tiled images"), "{fancy}");
        assert!(fancy.contains("survey pass"), "{fancy}");
        assert!(fancy.contains("already extracted"), "{fancy}");
    }

    // ── Render quality ──────────────────────────────────────────────────

    #[test]
    fn final_quality_matches_the_previous_hardcoded_encode() {
        // Any drift here silently changes what users ship.
        assert_eq!(RenderQuality::Final.crf(), "18");
        assert_eq!(RenderQuality::Final.preset(), "medium");
        assert_eq!(
            RenderQuality::Final.max_height(),
            None,
            "the deliverable must never be downscaled"
        );

        assert_eq!(RenderQuality::default(), RenderQuality::Final);
    }

    #[test]
    fn preview_tiers_are_progressively_cheaper() {
        let crf = |q: RenderQuality| q.crf().parse::<u32>().unwrap();
        assert!(crf(RenderQuality::Final) < crf(RenderQuality::Preview));
        assert!(crf(RenderQuality::Preview) < crf(RenderQuality::Draft));
        assert!(RenderQuality::Preview.max_height() > RenderQuality::Draft.max_height());
    }
}
