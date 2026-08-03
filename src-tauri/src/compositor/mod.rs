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

pub mod audio;
pub mod decoder;
pub mod effects;
pub mod encoder;
pub mod keyframe;

use std::path::Path;

use tiny_skia::Pixmap;

use crate::error::NarratorError;
use crate::models::RenderQuality;
use crate::video_edit::{EditClip, OverlayEffect, SpotlightData, VideoEditPlan};
use crate::video_engine;

use self::effects::text::TextRenderCache;
use self::keyframe::window_progress;

const MAX_OUTPUT_FPS: f64 = 60.0;
const SUPPORTED_EFFECTS: &[&str] = &["spotlight", "blur", "text", "fade", "zoom-pan"];

/// Compose all active effects for one frame onto `canvas`.
///
/// Runs a two-pass render to keep effect order independent of the array
/// order `effects` arrives in:
///
///   1. **Transform pass.** Any active `zoom-pan` effects apply first.
///      Zoom-pan clears the canvas and repaints from `source`, so running
///      it after an overlay would erase the overlay. Previously the render
///      order was whatever order the user added the effects in — a
///      spotlight added before a zoom-pan at the same time window would
///      silently disappear at export.
///   2. **Overlay pass.** Everything else (spotlight, blur, text, fade)
///      composites onto the transformed canvas in declaration order. Order
///      among overlays is still user-visible (e.g. fade-over-text vs
///      text-over-fade).
fn compose_frame(
    canvas: &mut Pixmap,
    source: &Pixmap,
    time: f32,
    effects: &[OverlayEffect],
    text_cache: &TextRenderCache,
) {
    // ── Pass 1: transforms ────────────────────────────────────────────────
    let mut zoom_pan_applied = false;
    for effect in effects {
        if effect.effect_type != "zoom-pan" {
            continue;
        }
        let Some(progress) = active_progress(effect, time) else {
            continue;
        };
        if let Some(zp) = &effect.zoom_pan {
            effects::zoom_pan::apply_zoom_pan(canvas, source, zp, progress);
            zoom_pan_applied = true;
        }
    }

    // If multiple zoom-pans were active, last-one-wins (matches previous
    // behaviour). `zoom_pan_applied` is only a marker in case a future
    // caller wants to know whether the canvas still contains the raw
    // source copy the caller set up, vs a transformed image.
    let _ = zoom_pan_applied;

    // ── Pass 2: overlays ──────────────────────────────────────────────────
    for effect in effects {
        if effect.effect_type == "zoom-pan" {
            continue;
        }
        let Some(progress) = active_progress(effect, time) else {
            continue;
        };
        // Ease the overlay-effect alpha to match the preview, which applies a
        // hard-coded ease-out to spotlight/blur/text/fade opacity (easing.ts
        // effectOpacity). `window_progress` returns a linear ramp by design
        // (zoom-pan eases downstream in apply_zoom_pan); this overlay pass is
        // the one consumer that must ease here or fades look different in the
        // export than in the preview.
        let effect_alpha = keyframe::ease(progress, keyframe::Interp::EaseOut);

        match effect.effect_type.as_str() {
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

/// Resolve an effect's `(start_time, end_time, transitions, reverse)` into a
/// `Some(progress in 0..1)` iff the effect is active at `time`, otherwise
/// `None`. Extracted so both the transform pass and the overlay pass share
/// the same activation math — keeps them from drifting.
fn active_progress(effect: &OverlayEffect, time: f32) -> Option<f32> {
    let start = effect.start_time as f32;
    let end = effect.end_time as f32;
    let t_in = effect.transition_in.unwrap_or(0.0) as f32;
    let t_out = effect.transition_out.unwrap_or(0.0) as f32;
    let reverse = effect.reverse.unwrap_or(false);
    // window_progress takes transitions in seconds (matches the frontend
    // TimelineEffect schema 1:1 — user enters seconds in the UI).
    window_progress(time, start, end, t_in, t_out, reverse)
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

// ── Phase 4: single-pass pipeline ──────────────────────────────────────────

/// Per-clip output duration on the timeline (after speed compression /
/// expansion, freeze override, etc).
fn clip_output_duration(clip: &EditClip) -> f64 {
    match clip.clip_type.as_deref() {
        Some("freeze") => clip.freeze_duration.unwrap_or(3.0).max(0.001),
        Some("image") => clip.image_duration.unwrap_or(3.0).max(0.001),
        _ => {
            let src = (clip.end_seconds - clip.start_seconds).max(0.001);
            src / clip.speed.max(0.01)
        }
    }
}

/// End-to-end render: clips + effects → single MP4. Replaces the per-clip
/// lossless re-encode + concat-demuxer + effects-pass pipeline that lived
/// in `video_edit::apply_edits`. The compositor's public surface (this
/// function) does the entire decode → composite → encode in one walk.
///
/// All "Reinitializing filters" failure modes are gone: there is no
/// time-varying ffmpeg filtergraph at any layer.
/// Fit `(width, height)` under `max_height`, preserving aspect ratio.
///
/// Both results are forced even: libx264 with yuv420p requires even dimensions,
/// and an odd value is a hard encode failure rather than a rounding artifact.
/// Never upscales — a 480p source rendered at "preview" stays 480p.
pub(crate) fn scaled_dimensions(width: u32, height: u32, max_height: Option<u32>) -> (u32, u32) {
    let width = width.max(2);
    let height = height.max(2);
    let Some(cap) = max_height else {
        return (even(width), even(height));
    };
    if height <= cap {
        return (even(width), even(height));
    }
    let scale = cap as f64 / height as f64;
    let scaled_w = ((width as f64) * scale).round() as u32;
    (even(scaled_w.max(2)), even(cap.max(2)))
}

/// Round down to the nearest even number, with a floor of 2.
fn even(v: u32) -> u32 {
    if v < 2 {
        2
    } else {
        v - (v % 2)
    }
}

pub async fn run_pipeline(
    input_path: &Path,
    output_path: &Path,
    plan: &VideoEditPlan,
    quality: RenderQuality,
    on_progress: &(impl Fn(f64, Option<String>) + Send + Sync),
    cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Result<(), NarratorError> {
    if plan.clips.is_empty() {
        return Err(NarratorError::ExportError("No clips to process".into()));
    }
    crate::cancel::check_cancelled(&cancel)?;

    let meta = video_engine::probe_video(input_path).await?;
    // Preview tiers downscale, which is what actually makes them fast on a 4K
    // source — a cheaper CRF alone still pays full-resolution decode, effects,
    // and compositing per frame. `Final` returns None and keeps source
    // resolution, so the deliverable is never silently degraded.
    let (width, height) = scaled_dimensions(meta.width, meta.height, quality.max_height());
    let fps = if meta.fps > 0.0 && meta.fps.is_finite() {
        meta.fps.min(MAX_OUTPUT_FPS)
    } else {
        30.0
    };

    // Compute per-clip start times on the output timeline.
    let mut clip_starts: Vec<f64> = Vec::with_capacity(plan.clips.len());
    let mut t = 0.0_f64;
    let mut total_duration = 0.0_f64;
    for clip in &plan.clips {
        clip_starts.push(t);
        let d = clip_output_duration(clip);
        t += d;
        total_duration += d;
    }
    let total_frames = (total_duration * fps).round().max(1.0) as u64;

    // Render the timeline audio first (concat + atempo per clip). The encoder
    // needs the WAV ready as its second input so the mux happens in one pass.
    let temp_audio_path = output_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("_audio_{}.wav", uuid::Uuid::new_v4()));
    let audio_path =
        audio::render_timeline_audio(input_path, &plan.clips, &temp_audio_path).await?;
    // Ensure the timeline WAV is removed on ANY exit (error, cancel, success),
    // not just the old success-only cleanup path below.
    let _wav_guard = audio_path.clone().map(|p| audio::ScopedRemove(vec![p]));

    // Pre-render text overlays once.
    let mut text_cache = TextRenderCache::default();
    let supported_effects: Vec<OverlayEffect> = plan
        .effects
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .filter(|e| SUPPORTED_EFFECTS.contains(&e.effect_type.as_str()))
        .cloned()
        .collect();
    for effect in &supported_effects {
        if effect.effect_type == "text" {
            if let Some(td) = &effect.text {
                // None = drawtext unavailable; warning already logged.
                let _ = text_cache.get_or_render(td, width, height).await?;
            }
        }
    }

    // Start one encoder for the whole render. Audio is muxed via -c:a copy
    // (PCM WAV → AAC happens automatically in the encoder by default; we
    // pass `-c:a aac` here too for explicitness).
    let mut encoder = encoder::Encoder::start_with_aac(
        output_path,
        width,
        height,
        fps,
        audio_path.as_deref(),
        quality,
    )
    .await?;

    let mut canvas = Pixmap::new(width, height)
        .ok_or_else(|| NarratorError::ExportError(format!("canvas alloc {width}x{height}")))?;
    let mut source_pix = Pixmap::new(width, height)
        .ok_or_else(|| NarratorError::ExportError(format!("source alloc {width}x{height}")))?;
    let mut last_decoded = vec![0u8; (width as usize) * (height as usize) * 4];
    let mut total_emitted: u64 = 0;

    let total_clips = plan.clips.len();
    for (clip_idx, clip) in plan.clips.iter().enumerate() {
        crate::cancel::check_cancelled(&cancel)?;
        let clip_start = clip_starts[clip_idx];
        let out_dur = clip_output_duration(clip);
        let out_frames = (out_dur * fps).round().max(1.0) as u64;

        // Announce the new clip once at its boundary so the UI shows a
        // per-clip label. Intra-clip ticks below forward `None` so they
        // inherit this label.
        let clip_start_pct = (total_emitted as f64 / total_frames as f64).clamp(0.0, 1.0) * 100.0;
        let clip_label = if clip.clip_type.as_deref() == Some("freeze") {
            format!("Creating freeze frame {} of {}", clip_idx + 1, total_clips)
        } else {
            format!("Processing clip {} of {}", clip_idx + 1, total_clips)
        };
        on_progress(clip_start_pct, Some(clip_label));

        // Per-clip source path. Clips appended via "+" carry their own
        // `input_path`; the primary clip leaves it None and inherits the
        // pipeline's default `input_path`.
        let clip_input: &Path = clip
            .input_path
            .as_deref()
            .map(Path::new)
            .unwrap_or(input_path);

        if clip.clip_type.as_deref() == Some("image") {
            // Image clip: decode the still once and repeat for the image's
            // output duration. The image path lives on the clip itself;
            // `start_seconds` is ignored (no time axis).
            let still = decoder::decode_single_frame_rgba(clip_input, 0.0, width, height).await?;
            last_decoded.copy_from_slice(&still);
            for f in 0..out_frames {
                source_pix.data_mut().copy_from_slice(&last_decoded);
                canvas.data_mut().copy_from_slice(source_pix.data());

                if let Some(zp) = &clip.zoom_pan {
                    let p = (f as f32) / (out_frames as f32).max(1.0);
                    effects::zoom_pan::apply_zoom_pan(&mut canvas, &source_pix, zp, p);
                    source_pix.data_mut().copy_from_slice(canvas.data());
                }

                let global_t = clip_start as f32 + (f as f32 / fps as f32);
                compose_frame(
                    &mut canvas,
                    &source_pix,
                    global_t,
                    &supported_effects,
                    &text_cache,
                );

                encoder.write_frame(canvas.data()).await?;
                total_emitted += 1;
                if total_emitted.is_multiple_of(8) {
                    let pct = (total_emitted as f64 / total_frames as f64).clamp(0.0, 1.0) * 100.0;
                    on_progress(pct, None);
                }
            }
        } else if clip.clip_type.as_deref() == Some("freeze") {
            // One source frame, repeated `out_frames` times. Per-clip
            // zoom-pan and overlay effects still animate over the duration.
            let frame_time = clip.freeze_source_time.unwrap_or(clip.start_seconds);
            let still =
                decoder::decode_single_frame_rgba(clip_input, frame_time, width, height).await?;
            last_decoded.copy_from_slice(&still);
            for f in 0..out_frames {
                source_pix.data_mut().copy_from_slice(&last_decoded);
                canvas.data_mut().copy_from_slice(source_pix.data());

                // Per-clip zoom-pan over this clip's window.
                if let Some(zp) = &clip.zoom_pan {
                    let p = (f as f32) / (out_frames as f32).max(1.0);
                    effects::zoom_pan::apply_zoom_pan(&mut canvas, &source_pix, zp, p);
                    // Update source so overlay effects see the post-clip frame.
                    source_pix.data_mut().copy_from_slice(canvas.data());
                }

                let global_t = clip_start as f32 + (f as f32 / fps as f32);
                compose_frame(
                    &mut canvas,
                    &source_pix,
                    global_t,
                    &supported_effects,
                    &text_cache,
                );

                encoder.write_frame(canvas.data()).await?;
                total_emitted += 1;
                if total_emitted.is_multiple_of(8) {
                    let pct = (total_emitted as f64 / total_frames as f64).clamp(0.0, 1.0) * 100.0;
                    on_progress(pct, None);
                }
            }
        } else {
            // We consume exactly `out_frames = out_dur * fps` output frames
            // and decode the source range [clip.start, clip.end] (length
            // `src_dur = end - start` seconds). For frame counts to match
            // (so every output frame maps to the correct source moment):
            //
            //   decoded_frames  = src_dur * decode_fps
            //   output_frames   = (src_dur / speed) * fps
            //   ⇒  decode_fps   = fps / speed
            //
            // Speed > 1 ⇒ fewer decoded frames (ffmpeg drops source frames);
            // speed < 1 ⇒ more decoded frames (ffmpeg duplicates them).
            //
            // An earlier version inverted this (fps * speed), which caused
            // video to show a fraction of the source at the wrong rate and
            // desynchronize from audio (`atempo` in compositor::audio is
            // correct, so the two tracks drift apart). Covered by
            // `integration_speed_2x_halves_duration` and friends.
            let speed = clip.speed.max(0.01);
            let decode_fps = (fps / speed).clamp(1.0, MAX_OUTPUT_FPS * 4.0);
            let (mut rx, decoder_handle) = decoder::decode_video_range(
                clip_input,
                clip.start_seconds,
                clip.end_seconds,
                width,
                height,
                decode_fps,
            )
            .await?;

            for f in 0..out_frames {
                // Mid-clip cancel check: returning here drops `rx`, so the
                // decoder task kills its ffmpeg, and the encoder's Drop removes
                // the partial output. Checked every 8 frames to stay cheap.
                if f.is_multiple_of(8) {
                    crate::cancel::check_cancelled(&cancel)?;
                }
                let frame = match rx.recv().await {
                    Some(fr) => fr,
                    None => {
                        // Source exhausted early (rounding / seek slop).
                        // Duplicate the last decoded frame to keep the
                        // output's frame count exact.
                        source_pix.data_mut().copy_from_slice(&last_decoded);
                        canvas.data_mut().copy_from_slice(source_pix.data());
                        if let Some(zp) = &clip.zoom_pan {
                            let p = (f as f32) / (out_frames as f32).max(1.0);
                            effects::zoom_pan::apply_zoom_pan(&mut canvas, &source_pix, zp, p);
                            source_pix.data_mut().copy_from_slice(canvas.data());
                        }
                        let global_t = clip_start as f32 + (f as f32 / fps as f32);
                        compose_frame(
                            &mut canvas,
                            &source_pix,
                            global_t,
                            &supported_effects,
                            &text_cache,
                        );
                        encoder.write_frame(canvas.data()).await?;
                        total_emitted += 1;
                        continue;
                    }
                };
                let expected = (width as usize) * (height as usize) * 4;
                if frame.data.len() != expected {
                    return Err(NarratorError::FfmpegFailed(format!(
                        "decoder yielded {} bytes, expected {expected}",
                        frame.data.len()
                    )));
                }
                last_decoded.copy_from_slice(&frame.data);
                source_pix.data_mut().copy_from_slice(&last_decoded);
                canvas.data_mut().copy_from_slice(source_pix.data());

                if let Some(zp) = &clip.zoom_pan {
                    let p = (f as f32) / (out_frames as f32).max(1.0);
                    effects::zoom_pan::apply_zoom_pan(&mut canvas, &source_pix, zp, p);
                    source_pix.data_mut().copy_from_slice(canvas.data());
                }

                let global_t = clip_start as f32 + (f as f32 / fps as f32);
                compose_frame(
                    &mut canvas,
                    &source_pix,
                    global_t,
                    &supported_effects,
                    &text_cache,
                );

                encoder.write_frame(canvas.data()).await?;
                total_emitted += 1;
                if total_emitted.is_multiple_of(8) {
                    let pct = (total_emitted as f64 / total_frames as f64).clamp(0.0, 1.0) * 100.0;
                    on_progress(pct, None);
                }
            }

            // Drain any frames the decoder is still trying to push and
            // collect its exit status.
            while rx.recv().await.is_some() { /* discard */ }
            decoder_handle
                .await
                .map_err(|e| NarratorError::FfmpegFailed(format!("decoder join: {e}")))??;
        }
    }

    encoder.finish().await?;
    // `_wav_guard` removes the timeline WAV on drop.

    on_progress(100.0, None);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{EasingPreset, RenderQuality, ZoomPanEffect, ZoomRegion};

    // ── Render-quality scaling ──────────────────────────────────────────

    #[test]
    fn final_quality_keeps_source_resolution() {
        // The deliverable must never be silently downscaled.
        assert_eq!(
            scaled_dimensions(3840, 2160, RenderQuality::Final.max_height()),
            (3840, 2160)
        );
        assert_eq!(
            scaled_dimensions(1920, 1080, RenderQuality::Final.max_height()),
            (1920, 1080)
        );
    }

    #[test]
    fn preview_downscales_4k_to_720p_preserving_aspect() {
        let (w, h) = scaled_dimensions(3840, 2160, RenderQuality::Preview.max_height());
        assert_eq!(h, 720);
        assert_eq!(w, 1280, "16:9 must stay 16:9");
    }

    #[test]
    fn preview_never_upscales_a_small_source() {
        // A 480p source at "preview" stays 480p rather than being blown up.
        assert_eq!(
            scaled_dimensions(854, 480, RenderQuality::Preview.max_height()),
            (854, 480)
        );
    }

    #[test]
    fn scaled_dimensions_are_always_even() {
        // libx264 with yuv420p rejects odd dimensions outright — this is an
        // encode failure, not a rounding artifact.
        for (w, h) in [(1919, 1081), (1081, 1919), (3, 3), (721, 405)] {
            for cap in [None, Some(720), Some(480)] {
                let (sw, sh) = scaled_dimensions(w, h, cap);
                assert_eq!(sw % 2, 0, "width {sw} odd for {w}x{h} cap {cap:?}");
                assert_eq!(sh % 2, 0, "height {sh} odd for {w}x{h} cap {cap:?}");
                assert!(sw >= 2 && sh >= 2);
            }
        }
    }

    #[test]
    fn portrait_sources_scale_by_height_too() {
        // 1080x1920 phone capture → 405x720, which then evens to 404x720.
        let (w, h) = scaled_dimensions(1080, 1920, Some(720));
        assert_eq!(h, 720);
        assert!((w as i32 - 404).abs() <= 2, "expected ~404, got {w}");
    }

    #[test]
    fn degenerate_dimensions_do_not_produce_a_zero_size() {
        assert_eq!(scaled_dimensions(0, 0, None), (2, 2));
        assert_eq!(scaled_dimensions(1, 1, Some(720)), (2, 2));
    }

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> Pixmap {
        let mut p = Pixmap::new(w, h).unwrap();
        let d = p.data_mut();
        for chunk in d.chunks_exact_mut(4) {
            chunk.copy_from_slice(&rgba);
        }
        p
    }

    fn spotlight_effect(start: f64, end: f64, x: f64, y: f64, radius: f64) -> OverlayEffect {
        OverlayEffect {
            effect_type: "spotlight".into(),
            start_time: start,
            end_time: end,
            transition_in: None,
            transition_out: None,
            reverse: None,
            spotlight: Some(SpotlightData {
                x,
                y,
                radius,
                dim_opacity: 0.8,
            }),
            blur: None,
            text: None,
            fade: None,
            zoom_pan: None,
        }
    }

    fn zoom_pan_effect(start: f64, end: f64) -> OverlayEffect {
        OverlayEffect {
            effect_type: "zoom-pan".into(),
            start_time: start,
            end_time: end,
            transition_in: None,
            transition_out: None,
            reverse: None,
            spotlight: None,
            blur: None,
            text: None,
            fade: None,
            zoom_pan: Some(ZoomPanEffect {
                start_region: ZoomRegion {
                    x: 0.0,
                    y: 0.0,
                    width: 0.5,
                    height: 0.5,
                },
                end_region: ZoomRegion {
                    x: 0.0,
                    y: 0.0,
                    width: 0.5,
                    height: 0.5,
                },
                easing: EasingPreset::Linear,
            }),
        }
    }

    /// Regression guard for the ordering bug: when spotlight comes BEFORE
    /// zoom-pan in the effects array and their time windows overlap, the
    /// old single-pass loop applied spotlight, then let zoom-pan clear the
    /// canvas to black — spotlight invisible. After the two-pass split,
    /// zoom-pan runs first regardless of array position, so both orderings
    /// produce identical frames.
    #[test]
    fn compose_frame_order_independent_for_zoom_then_spotlight() {
        let text_cache = TextRenderCache::default();
        let source = solid(40, 40, [200, 50, 50, 255]);

        let sp = spotlight_effect(0.0, 2.0, 0.5, 0.5, 0.25);
        let zp = zoom_pan_effect(0.0, 2.0);

        let mut canvas_a = source.clone();
        compose_frame(
            &mut canvas_a,
            &source,
            1.0,
            &[sp.clone(), zp.clone()],
            &text_cache,
        );

        let mut canvas_b = source.clone();
        compose_frame(&mut canvas_b, &source, 1.0, &[zp, sp], &text_cache);

        assert_eq!(
            canvas_a.data(),
            canvas_b.data(),
            "spotlight-before-zoompan and zoompan-before-spotlight must produce identical frames"
        );
    }

    /// Guard against a silent regression to the old behaviour: with the bug,
    /// `[spotlight, zoom-pan]` produced a canvas that was just the zoomed
    /// source (spotlight wiped). After the fix both orderings have the dim
    /// layer, so neither canvas matches a naked zoom-pan render.
    #[test]
    fn compose_frame_spotlight_survives_when_ordered_before_zoom_pan() {
        let text_cache = TextRenderCache::default();
        let source = solid(40, 40, [200, 50, 50, 255]);

        let sp = spotlight_effect(0.0, 2.0, 0.5, 0.5, 0.25);
        let zp = zoom_pan_effect(0.0, 2.0);

        let mut with_spotlight_first = source.clone();
        compose_frame(
            &mut with_spotlight_first,
            &source,
            1.0,
            &[sp, zp.clone()],
            &text_cache,
        );

        let mut zoom_only = source.clone();
        compose_frame(&mut zoom_only, &source, 1.0, &[zp], &text_cache);

        assert_ne!(
            with_spotlight_first.data(),
            zoom_only.data(),
            "spotlight-before-zoompan must not be wiped to a plain zoom-pan render"
        );
    }

    /// When an effect's time window doesn't include `time`, it contributes
    /// nothing to the frame. Regression guard so a future refactor doesn't
    /// accidentally always-apply zoom-pan.
    #[test]
    fn compose_frame_ignores_inactive_effects() {
        let text_cache = TextRenderCache::default();
        let source = solid(40, 40, [100, 100, 100, 255]);

        let zp_future = zoom_pan_effect(5.0, 7.0);
        let sp_past = spotlight_effect(0.0, 1.0, 0.5, 0.5, 0.3);

        let mut canvas = source.clone();
        compose_frame(
            &mut canvas,
            &source,
            3.0,
            &[zp_future, sp_past],
            &text_cache,
        );

        assert_eq!(
            canvas.data(),
            source.data(),
            "no effect active at time=3s → canvas must equal the source"
        );
    }
}
