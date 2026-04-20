//! Transport-agnostic rendering API.
//!
//! Wraps `video_edit` and `video_engine` so callers (Tauri commands, CLI) plug
//! progress events into a `ProgressReporter` trait instead of hand-rolling a
//! closure each time. The functions themselves are unchanged — this is the
//! shim layer that lets the CLI emit NDJSON while the GUI uses a Tauri channel.
//!
//! Add new render-shaped capabilities here, not directly in `commands.rs`,
//! so they're reachable from both transports for free.
//!
//! Owns no state; every function is callable from any thread.
//!
//! Note on the new compositor (Phase 3+): `apply_edits` here delegates to the
//! in-process Rust compositor (`crate::compositor::run_pipeline`) when the
//! edit plan contains overlay effects, falling back to the legacy ffmpeg
//! `filter_complex` path only for plain trim/concat without effects.
//! The frontend `VideoEditPlan` IPC contract is unchanged.
//!
//! The fallback exists strictly so the cut-over is reversible per phase; it is
//! removed in Phase 6.
//!
//! Authors: keep this module tiny. If a function grows logic beyond
//! "decide reporter / call existing pure fn / forward result", that logic
//! belongs in the underlying module.

use std::path::Path;
use std::sync::Arc;

use crate::error::NarratorError;
use crate::models::{ProgressEvent, VideoMetadata};
use crate::{video_edit, video_engine};

pub use video_edit::{MergeOutcome, SubtitleStyle, VideoEditPlan};

/// Bridges progress events from a render to a transport (Tauri channel,
/// stderr NDJSON, in-memory buffer for tests, etc.).
///
/// Implementations must be cheap to clone via `Arc`; reporters are commonly
/// captured by `move` closures inside async tasks.
pub trait ProgressReporter: Send + Sync {
    fn report(&self, event: ProgressEvent);
}

/// Discards every event. For tests and one-shot calls where the caller does
/// not care about progress.
#[allow(dead_code)]
pub struct NoopReporter;

impl ProgressReporter for NoopReporter {
    fn report(&self, _event: ProgressEvent) {}
}

/// Wraps any `Fn(ProgressEvent)` so callers can build a reporter inline
/// without defining a struct. Used by the Tauri command shims.
pub struct FnReporter<F>(pub F)
where
    F: Fn(ProgressEvent) + Send + Sync + 'static;

impl<F> ProgressReporter for FnReporter<F>
where
    F: Fn(ProgressEvent) + Send + Sync + 'static,
{
    fn report(&self, event: ProgressEvent) {
        (self.0)(event);
    }
}

/// Helper for callers that only care about percent updates.
#[allow(dead_code)]
fn forward_percent(reporter: &Arc<dyn ProgressReporter>) -> impl Fn(f64) + Send + Sync + use<'_> {
    let reporter = reporter.clone();
    move |percent| reporter.report(ProgressEvent::progress(percent))
}

/// Helper for callers that emit `(percent, message)` pairs. Use this when
/// the producer wants to attach a human-readable sub-label at milestones
/// (e.g. "Processing clip 2 of 5", "Combining clips"). For plain ticks, pass
/// `None` and the UI will keep the current label.
fn forward_percent_msg(
    reporter: &Arc<dyn ProgressReporter>,
) -> impl Fn(f64, Option<String>) + Send + Sync + use<'_> {
    let reporter = reporter.clone();
    move |percent, message| reporter.report(ProgressEvent::Progress { percent, message })
}

// ── Public API ─────────────────────────────────────────────────────────────

/// Probe a video for resolution / fps / duration / codec / file size.
#[allow(dead_code)]
pub async fn probe_video(path: &Path) -> Result<VideoMetadata, NarratorError> {
    video_engine::probe_video(path).await
}

/// Apply an edit plan (clips + overlay effects) and write a single MP4 to
/// `output_path`. See `VideoEditPlan` for the full schema.
pub async fn apply_edits(
    input_path: &str,
    output_path: &str,
    plan: &VideoEditPlan,
    reporter: Arc<dyn ProgressReporter>,
) -> Result<String, NarratorError> {
    let on_progress = forward_percent_msg(&reporter);
    video_edit::apply_edits(input_path, output_path, plan, on_progress).await
}

/// Mux narration audio into an existing video. `replace_audio = true` swaps
/// the audio track wholesale; `false` mixes original + narration with
/// auto-ducking (`duck_db` controls the dip, typical range -4..-15 dB).
/// Returns a `MergeOutcome` so callers can detect the narration-only
/// fallback (e.g. to warn the user the source had no audio).
pub async fn merge_audio_video(
    video_path: &str,
    audio_path: &str,
    output_path: &str,
    replace_audio: bool,
    duck_db: f32,
    reporter: Arc<dyn ProgressReporter>,
) -> Result<MergeOutcome, NarratorError> {
    let on_progress = forward_percent_msg(&reporter);
    video_edit::merge_audio_video(
        video_path,
        audio_path,
        output_path,
        replace_audio,
        duck_db,
        on_progress,
    )
    .await
}

/// Burn an SRT into a video as hard-subtitles using libass.
pub async fn burn_subtitles(
    video_path: &str,
    srt_path: &str,
    output_path: &str,
    style: &SubtitleStyle,
    reporter: Arc<dyn ProgressReporter>,
) -> Result<String, NarratorError> {
    let on_progress = forward_percent_msg(&reporter);
    video_edit::burn_subtitles(video_path, srt_path, output_path, style, on_progress).await
}

/// Extract a single JPEG/PNG frame at `timestamp` (seconds).
pub async fn extract_single_frame(
    video_path: &str,
    timestamp: f64,
    output_path: &str,
) -> Result<String, NarratorError> {
    video_edit::extract_single_frame(video_path, timestamp, output_path).await
}

/// Generate `count` evenly-spaced thumbnail JPGs into `output_dir`.
/// Returns sorted file paths.
pub async fn extract_edit_thumbnails(
    video_path: &str,
    output_dir: &str,
    count: usize,
) -> Result<Vec<String>, NarratorError> {
    video_edit::extract_edit_thumbnails(video_path, output_dir, count).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn noop_reporter_is_silent() {
        let r: Arc<dyn ProgressReporter> = Arc::new(NoopReporter);
        r.report(ProgressEvent::progress(42.0));
        // Nothing to assert — just shouldn't panic.
    }

    #[test]
    fn fn_reporter_forwards_events() {
        let count = Arc::new(AtomicUsize::new(0));
        let count_inner = count.clone();
        let r: Arc<dyn ProgressReporter> = Arc::new(FnReporter(move |_e| {
            count_inner.fetch_add(1, Ordering::SeqCst);
        }));
        r.report(ProgressEvent::progress(10.0));
        r.report(ProgressEvent::progress(20.0));
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn forward_percent_msg_wraps_into_progress_event() {
        use std::sync::Mutex;
        let events: Arc<Mutex<Vec<ProgressEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let reporter: Arc<dyn ProgressReporter> = Arc::new(FnReporter(move |e| {
            sink.lock().unwrap().push(e);
        }));
        let forward = forward_percent_msg(&reporter);
        forward(12.0, None);
        forward(34.5, Some("Processing clip 2 of 5".to_string()));

        let captured = events.lock().unwrap();
        assert_eq!(captured.len(), 2);
        match &captured[0] {
            ProgressEvent::Progress { percent, message } => {
                assert_eq!(*percent, 12.0);
                assert!(message.is_none());
            }
            other => panic!("expected Progress, got {:?}", other),
        }
        match &captured[1] {
            ProgressEvent::Progress { percent, message } => {
                assert_eq!(*percent, 34.5);
                assert_eq!(message.as_deref(), Some("Processing clip 2 of 5"));
            }
            other => panic!("expected Progress, got {:?}", other),
        }
    }
}
