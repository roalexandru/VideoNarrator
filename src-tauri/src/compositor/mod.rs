//! In-process Rust compositor that replaces the time-varying ffmpeg
//! filtergraph for overlay effects.
//!
//! Architecture (Phase 3 wedge):
//!   1. ffmpeg decodes the **already-concatenated** input MP4 to raw RGBA
//!      frames at the project's resolution + fps (`decoder::decode_video`).
//!   2. For each output frame N we resolve which overlay effects are active
//!      at time = N / fps, evaluate keyframe progress + transition alpha
//!      for each, and compose them in declaration order onto the canvas.
//!   3. ffmpeg encodes the composited RGBA stream + copies the original
//!      audio track into the output MP4 (`encoder::Encoder`).
//!
//! Phase 4 will replace the per-clip + concat ffmpeg passes too, so the
//! decoder takes raw clips and the concatenation happens by switching
//! source streams inside the compose loop. This module's public surface
//! does not change between phases — only `apply_overlay_effects`'s caller
//! flips from "concat then call us" to "call us with the clip list".
//!
//! The "Reinitializing filters" class of bugs is impossible here by
//! construction — there is no time-varying ffmpeg filtergraph anywhere
//! in this path.

pub mod decoder;
pub mod effects;
pub mod encoder;
pub mod keyframe;

use std::path::Path;

use tiny_skia::Pixmap;

use crate::error::NarratorError;
use crate::video_edit::{OverlayEffect, SpotlightData};
use crate::video_engine;

use self::effects::text::TextRenderCache;
use self::keyframe::window_progress;

/// Apply all overlay effects in `effects` to `input_path` and write the
/// composited MP4 to `output_path`. Audio from the input is copied through.
///
/// `on_progress` receives a 0..100 float — the caller is expected to scale
/// it into whatever overall progress band this pass owns (e.g. 90..99).
///
/// This is the Phase-3 surface: callers (currently `video_edit::apply_edits`)
/// run the per-clip + concat ffmpeg passes first, then hand the concatenated
/// MP4 to us. We never call ffmpeg with a `filter_complex` that varies over
/// time.
pub async fn apply_overlay_effects(
    input_path: &Path,
    output_path: &Path,
    effects: &[OverlayEffect],
    on_progress: &(impl Fn(f64) + Send + Sync),
) -> Result<(), NarratorError> {
    if effects.is_empty() {
        return Err(NarratorError::ExportError(
            "compositor called with no effects".into(),
        ));
    }

    let meta = video_engine::probe_video(input_path).await?;
    let width = meta.width.max(2);
    let height = meta.height.max(2);
    let fps = if meta.fps > 0.0 && meta.fps.is_finite() {
        meta.fps.min(60.0)
    } else {
        30.0
    };
    let duration = meta.duration_seconds.max(0.001);
    let total_frames = (duration * fps).round() as u64;

    // Pre-render text overlays once.
    let mut text_cache = TextRenderCache::default();
    for effect in effects {
        if effect.effect_type == "text" {
            if let Some(td) = &effect.text {
                text_cache.get_or_render(td, width, height).await?;
            }
        }
    }

    let (mut frame_rx, decoder_handle) =
        decoder::decode_video(input_path, width, height, fps).await?;

    let mut encoder =
        encoder::Encoder::start(output_path, width, height, fps, Some(input_path)).await?;

    let mut frame_idx: u64 = 0;
    let mut canvas = Pixmap::new(width, height)
        .ok_or_else(|| NarratorError::ExportError(format!("canvas alloc {width}x{height}")))?;
    let mut source = Pixmap::new(width, height)
        .ok_or_else(|| NarratorError::ExportError(format!("source alloc {width}x{height}")))?;

    while let Some(frame) = frame_rx.recv().await {
        // Copy source frame into the source pixmap.
        let src_data = source.data_mut();
        // frame.data may be Arc<Vec<u8>> shared with channel — safe to copy.
        let expected = (width as usize) * (height as usize) * 4;
        if frame.data.len() != expected {
            return Err(NarratorError::FfmpegFailed(format!(
                "decoder yielded {} bytes, expected {expected}",
                frame.data.len()
            )));
        }
        src_data.copy_from_slice(&frame.data);

        // Reset canvas to source.
        canvas.data_mut().copy_from_slice(source.data());

        let time = frame_idx as f32 / fps as f32;
        compose_frame(&mut canvas, &source, time, effects, &text_cache);

        encoder.write_frame(canvas.data()).await?;

        frame_idx += 1;

        if total_frames > 0 && frame_idx.is_multiple_of(10) {
            let local = (frame_idx as f64 / total_frames as f64).clamp(0.0, 1.0);
            on_progress(local * 100.0);
        }
    }

    // Drain decoder.
    decoder_handle
        .await
        .map_err(|e| NarratorError::FfmpegFailed(format!("decoder join: {e}")))??;

    encoder.finish().await?;

    on_progress(100.0);
    Ok(())
}

/// Compose all active effects for one frame onto `canvas`.
fn compose_frame(
    canvas: &mut Pixmap,
    source: &Pixmap,
    time: f32,
    effects: &[OverlayEffect],
    text_cache: &TextRenderCache,
) {
    for effect in effects {
        let start = effect.start_time as f32;
        let end = effect.end_time as f32;
        let t_in = effect.transition_in.unwrap_or(0.0) as f32;
        let t_out = effect.transition_out.unwrap_or(0.0) as f32;
        let reverse = effect.reverse.unwrap_or(false);

        // Convert absolute transition seconds into a fraction of the window
        // (matches the existing video_edit semantics where the user enters
        // seconds, not a percentage).
        let dur = (end - start).max(f32::EPSILON);
        let progress = match window_progress(time, start, end, t_in / dur, t_out / dur, reverse) {
            Some(p) => p,
            None => continue,
        };

        // For most effects, `progress` *is* both the value used in the
        // animated parameters AND the alpha for transitions (since the
        // ramp shape is the same).
        let effect_alpha = progress;

        match effect.effect_type.as_str() {
            "zoom-pan" => {
                if let Some(zp) = &effect.zoom_pan {
                    // Zoom-pan is a transform of the source frame, not an
                    // overlay — overwrite the canvas wholesale.
                    effects::zoom_pan::apply_zoom_pan(canvas, source, zp, progress);
                }
            }
            "spotlight" => {
                if let Some(sp) = &effect.spotlight {
                    apply_spotlight_safe(canvas, sp, effect_alpha);
                }
            }
            "blur" => {
                if let Some(b) = &effect.blur {
                    effects::blur::apply_blur(
                        canvas,
                        b.x as f32,
                        b.y as f32,
                        b.width as f32,
                        b.height as f32,
                        b.radius as f32,
                        b.invert.unwrap_or(false),
                        effect_alpha,
                    );
                }
            }
            "text" => {
                if let Some(td) = &effect.text {
                    if let Some(pre) = text_cache.lookup(td, canvas.width(), canvas.height()) {
                        effects::text::apply_text(canvas, &pre.pixmap, effect_alpha);
                    }
                }
            }
            "fade" => {
                if let Some(f) = &effect.fade {
                    effects::fade::apply_fade(canvas, &f.color, f.opacity as f32 * effect_alpha);
                }
            }
            other => {
                // Unknown effect type — silently skip (forward-compat).
                let _ = other;
            }
        }
    }
}

fn apply_spotlight_safe(canvas: &mut Pixmap, sp: &SpotlightData, alpha: f32) {
    effects::spotlight::apply_spotlight(
        canvas,
        sp.x as f32,
        sp.y as f32,
        sp.radius as f32,
        sp.dim_opacity as f32,
        alpha,
    );
}
