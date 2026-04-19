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
/// a normalized progress value, applying transitionIn / transitionOut /
/// reverse semantics. Returns `None` when the time falls outside the
/// effect's effective window (so the caller can skip evaluation).
///
/// Mirrors `build_progress_expr` in `video_edit.rs` but in plain Rust.
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
    let dur = (end - start).max(f32::EPSILON);
    let local = (time - start) / dur;

    // Both transitions are expressed as a fraction of the window.
    let t_in = transition_in.clamp(0.0, 0.5);
    let t_out = transition_out.clamp(0.0, 0.5);

    let progress = if reverse {
        // Animate up then back down; transitions become alpha ramps elsewhere.
        if local < 0.5 {
            local * 2.0
        } else {
            (1.0 - local) * 2.0
        }
    } else if t_in > 0.0 && local < t_in {
        local / t_in.max(f32::EPSILON)
    } else if t_out > 0.0 && local > 1.0 - t_out {
        (1.0 - local) / t_out.max(f32::EPSILON)
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
    fn full_window_progress_is_one() {
        let p = window_progress(1.5, 1.0, 2.0, 0.0, 0.0, false).unwrap();
        assert!((p - 1.0).abs() < 1e-5);
    }

    #[test]
    fn transition_in_ramps_up() {
        // 1s window, 20% transition in
        let p_quarter = window_progress(1.05, 1.0, 2.0, 0.2, 0.0, false).unwrap();
        assert!(p_quarter < 1.0 && p_quarter > 0.0);
    }

    #[test]
    fn reverse_returns_to_zero() {
        let p_start = window_progress(1.0, 1.0, 2.0, 0.0, 0.0, true).unwrap();
        let p_mid = window_progress(1.5, 1.0, 2.0, 0.0, 0.0, true).unwrap();
        let p_end = window_progress(2.0, 1.0, 2.0, 0.0, 0.0, true).unwrap();
        assert!(p_start < 0.1);
        assert!((p_mid - 1.0).abs() < 0.1);
        assert!(p_end < 0.1);
    }
}
