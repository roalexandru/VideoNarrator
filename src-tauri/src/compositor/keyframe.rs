// `Keyframe` + `linear_segment` / `constant` / `value_at` / `ease` are the
// animated-parameter abstraction the compositor + Phase 5 audio gain ramps
// will consume. They mirror libopenshot::Keyframe / mlt_animation. Most are
// not yet used by the Phase 3 wedge — `window_progress` is. Allow until the
// compositor fully owns the timeline (Phase 4+).
#![allow(dead_code)]
//! Animated parameters with eased interpolation.
//!
//! Mirrors the design used by libopenshot (`Keyframe`) and MLT (`mlt_animation`):
//! every animatable property is read through this type each frame, returning
//! an interpolated scalar. Effects don't need to know what easing the caller
//! picked — they just call `value_at(t)`.
//!
//! Today the timeline / overlay model has only one keyframe per param-pair
//! (start/end) and a single easing preset (`crate::models::EasingPreset`).
//! We use this richer keyframe machinery anyway so adding multi-keyframe
//! animation later is a data change, not a code change.
//!
//! Implementation note: f32-only for now. tiny-skia uses f32 transforms,
//! and audio gain ramps (Phase 5) will likewise be f32.

use crate::models::EasingPreset;

/// A single (time → value) point in a keyframe sequence.
///
/// `time` is the *normalized* progress through the effect window (0.0 → 1.0).
/// The effect window itself maps to absolute timeline seconds outside.
#[derive(Debug, Clone, Copy)]
pub struct KeyPoint {
    pub time: f32,
    pub value: f32,
    pub interp: Interp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interp {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    /// Step interpolation (current value held until the next point).
    Hold,
}

impl From<EasingPreset> for Interp {
    fn from(p: EasingPreset) -> Self {
        match p {
            EasingPreset::Linear => Interp::Linear,
            EasingPreset::EaseIn => Interp::EaseIn,
            EasingPreset::EaseOut => Interp::EaseOut,
            EasingPreset::EaseInOut => Interp::EaseInOut,
        }
    }
}

/// Sequence of keyframe points. `value_at(t)` gives the eased value for
/// any normalized time, including before the first or after the last point.
#[derive(Debug, Clone, Default)]
pub struct Keyframe {
    pub points: Vec<KeyPoint>,
}

impl Keyframe {
    /// Common case: a 2-point keyframe (start at t=0, end at t=1) with one
    /// easing applied to the segment between them.
    pub fn linear_segment(start: f32, end: f32, interp: Interp) -> Self {
        Self {
            points: vec![
                KeyPoint {
                    time: 0.0,
                    value: start,
                    interp,
                },
                KeyPoint {
                    time: 1.0,
                    value: end,
                    interp,
                },
            ],
        }
    }

    /// Constant value across the whole window (no animation).
    pub fn constant(v: f32) -> Self {
        Self {
            points: vec![KeyPoint {
                time: 0.0,
                value: v,
                interp: Interp::Hold,
            }],
        }
    }

    /// Sample the eased value at normalized time `t`. Times outside `[0, 1]`
    /// clamp to the nearest endpoint (no extrapolation).
    pub fn value_at(&self, t: f32) -> f32 {
        if self.points.is_empty() {
            return 0.0;
        }
        if self.points.len() == 1 {
            return self.points[0].value;
        }
        if t <= self.points[0].time {
            return self.points[0].value;
        }
        if t >= self.points[self.points.len() - 1].time {
            return self.points[self.points.len() - 1].value;
        }

        // Find the surrounding pair.
        let mut i = 0;
        while i + 1 < self.points.len() && self.points[i + 1].time < t {
            i += 1;
        }
        let a = self.points[i];
        let b = self.points[i + 1];

        if (b.time - a.time).abs() < f32::EPSILON {
            return b.value;
        }
        let local = ((t - a.time) / (b.time - a.time)).clamp(0.0, 1.0);
        let eased = ease(local, a.interp);
        a.value + (b.value - a.value) * eased
    }
}

/// Apply an easing curve to linear progress in [0, 1].
pub fn ease(t: f32, interp: Interp) -> f32 {
    match interp {
        Interp::Linear => t,
        Interp::EaseIn => t * t,
        Interp::EaseOut => t * (2.0 - t),
        Interp::EaseInOut => {
            if t < 0.5 {
                2.0 * t * t
            } else {
                -1.0 + (4.0 - 2.0 * t) * t
            }
        }
        Interp::Hold => 0.0,
    }
}

/// Map an absolute timeline `time` (seconds) within an effect window into
/// a normalized animation progress (0..1), applying transition-in /
/// transition-out / reverse semantics. Returns `None` when the time falls
/// outside the window.
///
/// `transition_in` and `transition_out` are in **seconds** (not fractions).
/// The returned value is LINEAR — easing (`Linear` / `EaseIn` / `EaseOut`
/// / `EaseInOut`) is applied downstream (in `effects::zoom_pan`, etc.) so
/// tests here stay trivially checkable and effects can pick their own curve.
///
/// Shape (mirrors `src/features/edit-video/easing.ts::effectProgress` so
/// the preview and the exported render animate identically):
///
/// - **reverse = true**  (three phases: ramp-in → hold → ramp-out)
///   - `[0, transition_in)`          ramp 0 → 1
///   - `[transition_in, dur - transition_out]`  hold at 1
///   - `(dur - transition_out, dur]` ramp 1 → 0
///
///   Overlap (`transition_in + transition_out > dur`) is resolved by
///   `max(hold_start, hold_end)` — the in-ramp wins over the out-ramp.
///
/// - **reverse = false** (simple: ramp-in → hold)
///   - `transition_in` covers the whole window (or is 0) → full-window
///     linear ramp (`local / dur`). Lets the effect animate from start
///     to end across its full duration without the user spelling out a
///     transition.
///   - otherwise: ramp 0 → 1 over `transition_in`, then hold at 1
pub fn window_progress(
    time: f32,
    start: f32,
    end: f32,
    transition_in: f32,
    transition_out: f32,
    reverse: bool,
) -> Option<f32> {
    if time < start || time > end {
        return None;
    }
    let total_duration = (end - start).max(f32::EPSILON);
    let local_time = (time - start).max(0.0);

    let progress = if reverse {
        let hold_start = transition_in;
        let hold_end = total_duration - transition_out;

        if local_time <= 0.0 {
            0.0
        } else if transition_in > 0.0 && local_time < hold_start {
            local_time / transition_in
        } else if local_time <= hold_start.max(hold_end) {
            1.0
        } else if transition_out > 0.0 && local_time < total_duration {
            let out_progress = (local_time - hold_end) / transition_out;
            1.0 - out_progress
        } else {
            0.0
        }
    } else if transition_in > 0.0 && transition_in < total_duration && local_time < transition_in {
        local_time / transition_in
    } else if transition_in <= 0.0 || transition_in >= total_duration {
        // Full-window linear animation (no distinct transition).
        local_time / total_duration
    } else {
        1.0
    };
    Some(progress.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_segment_endpoints_exact() {
        let k = Keyframe::linear_segment(10.0, 20.0, Interp::Linear);
        assert!((k.value_at(0.0) - 10.0).abs() < 1e-5);
        assert!((k.value_at(1.0) - 20.0).abs() < 1e-5);
        assert!((k.value_at(0.5) - 15.0).abs() < 1e-5);
    }

    #[test]
    fn ease_in_out_symmetric() {
        let mid = ease(0.5, Interp::EaseInOut);
        assert!((mid - 0.5).abs() < 1e-3);
        // Ease-in-out is monotonic
        assert!(ease(0.25, Interp::EaseInOut) < 0.5);
        assert!(ease(0.75, Interp::EaseInOut) > 0.5);
    }

    #[test]
    fn constant_keyframe_ignores_time() {
        let k = Keyframe::constant(42.0);
        assert_eq!(k.value_at(0.0), 42.0);
        assert_eq!(k.value_at(0.7), 42.0);
        assert_eq!(k.value_at(1.0), 42.0);
    }

    #[test]
    fn out_of_window_returns_none() {
        assert!(window_progress(0.5, 1.0, 2.0, 0.0, 0.0, false).is_none());
        assert!(window_progress(2.5, 1.0, 2.0, 0.0, 0.0, false).is_none());
    }

    #[test]
    fn non_reverse_no_transition_animates_over_full_window() {
        // With transitionIn = 0 the whole window is a single ramp 0→1 —
        // matches easing.ts line 58-60 ("No transition or past it").
        let p_start = window_progress(0.0, 0.0, 10.0, 0.0, 0.0, false).unwrap();
        let p_mid = window_progress(5.0, 0.0, 10.0, 0.0, 0.0, false).unwrap();
        let p_end = window_progress(10.0, 0.0, 10.0, 0.0, 0.0, false).unwrap();
        assert!(p_start.abs() < 1e-4);
        assert!((p_mid - 0.5).abs() < 1e-4);
        assert!((p_end - 1.0).abs() < 1e-4);
    }

    #[test]
    fn non_reverse_with_transition_ramps_then_holds() {
        // 10s window, 2s transition-in → 0..2s ramps, 2..10s holds at 1.
        let p_0 = window_progress(0.0, 0.0, 10.0, 2.0, 0.0, false).unwrap();
        let p_1s = window_progress(1.0, 0.0, 10.0, 2.0, 0.0, false).unwrap();
        let p_5s = window_progress(5.0, 0.0, 10.0, 2.0, 0.0, false).unwrap();
        let p_end = window_progress(10.0, 0.0, 10.0, 2.0, 0.0, false).unwrap();
        assert!(p_0.abs() < 1e-4);
        assert!((p_1s - 0.5).abs() < 1e-4);
        assert!((p_5s - 1.0).abs() < 1e-4);
        assert!((p_end - 1.0).abs() < 1e-4);
    }

    #[test]
    fn reverse_three_phase_matches_frontend() {
        // The user's actual config: 17.9s window, 2s in, 3s out, reverse.
        // Expected: ramp 0→1 over 0..2s, hold 1 over 2..14.9s, ramp 1→0
        // over 14.9..17.9s — exactly what easing.ts::effectProgress does.
        let dur = 17.9_f32;
        let t_in = 2.0_f32;
        let t_out = 3.0_f32;
        let at = |t| window_progress(t, 0.0, dur, t_in, t_out, true).unwrap();

        // Ramp-in
        assert!(at(0.0).abs() < 1e-4, "at 0s, progress should be 0");
        assert!(
            (at(1.0) - 0.5).abs() < 1e-4,
            "1s into 2s ramp-in → 0.5, got {}",
            at(1.0)
        );
        assert!((at(1.999) - 0.9995).abs() < 1e-3, "end of ramp-in ≈ 1");

        // Hold
        assert!((at(2.0) - 1.0).abs() < 1e-4, "hold starts at 1.0");
        assert!((at(8.0) - 1.0).abs() < 1e-4, "middle of hold = 1.0");
        assert!((at(14.9) - 1.0).abs() < 1e-4, "hold ends at 1.0");

        // Ramp-out
        assert!(
            (at(16.4) - 0.5).abs() < 1e-3,
            "mid ramp-out (1.5s in) → 0.5, got {}",
            at(16.4)
        );
        assert!(at(17.9) < 1e-3, "at 17.9s, progress back to 0");
    }

    #[test]
    fn reverse_no_transitions_holds_full_window() {
        // reverse=true with both transitions=0 means there's nothing to
        // ramp — the effect is just "active" for the whole window. Must
        // hold at 1 (NOT the old triangle-wave behaviour).
        //
        // Matches easing.ts's `if (localTime <= 0) return 0;` guard at
        // the exact start — the first sub-frame of the window returns 0,
        // every subsequent frame returns 1. This parity with the preview
        // is why we test `p_any_interior` rather than the boundary.
        let p_any_interior = window_progress(0.001, 0.0, 2.0, 0.0, 0.0, true).unwrap();
        let p_mid = window_progress(1.0, 0.0, 2.0, 0.0, 0.0, true).unwrap();
        let p_end = window_progress(2.0, 0.0, 2.0, 0.0, 0.0, true).unwrap();
        assert!((p_any_interior - 1.0).abs() < 1e-4);
        assert!((p_mid - 1.0).abs() < 1e-4);
        assert!((p_end - 1.0).abs() < 1e-4);
    }

    #[test]
    fn reverse_overlapping_transitions_dont_panic() {
        // transitionIn + transitionOut > total — the in-ramp wins via the
        // `hold_start.max(hold_end)` branch. No panic, always in [0, 1].
        let dur = 3.0_f32;
        let at = |t| window_progress(t, 0.0, dur, 2.5, 2.0, true).unwrap();
        for t in [0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0] {
            let p = at(t);
            assert!((0.0..=1.0).contains(&p), "t={t} p={p} out of range");
        }
    }
}
