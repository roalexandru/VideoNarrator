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
use crate::video_edit::{EditClip, OverlayEffect, SpotlightData, VideoEditPlan};
use crate::video_engine;

use self::effects::text::TextRenderCache;
use self::keyframe::window_progress;

const MAX_OUTPUT_FPS: f64 = 60.0;
const SUPPORTED_EFFECTS: &[&str] = &["spotlight", "blur", "text", "fade", "zoom-pan"];

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

        // window_progress takes transitions in seconds (matches the frontend
        // TimelineEffect schema 1:1 — user enters seconds in the UI).
        let progress = match window_progress(time, start, end, t_in, t_out, reverse) {
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

// ── Phase 4: single-pass pipeline ──────────────────────────────────────────

/// Per-clip output duration on the timeline (after speed compression /
/// expansion, freeze override, etc).
fn clip_output_duration(clip: &EditClip) -> f64 {
    if clip.clip_type.as_deref() == Some("freeze") {
        clip.freeze_duration.unwrap_or(3.0).max(0.001)
    } else {
        let src = (clip.end_seconds - clip.start_seconds).max(0.001);
        src / clip.speed.max(0.01)
    }
}

/// End-to-end render: clips + effects → single MP4. Replaces the per-clip
/// lossless re-encode + concat-demuxer + effects-pass pipeline that lived
/// in `video_edit::apply_edits`. The compositor's public surface (this
/// function) does the entire decode → composite → encode in one walk.
///
/// All "Reinitializing filters" failure modes are gone: there is no
/// time-varying ffmpeg filtergraph at any layer.
pub async fn run_pipeline(
    input_path: &Path,
    output_path: &Path,
    plan: &VideoEditPlan,
    on_progress: &(impl Fn(f64) + Send + Sync),
) -> Result<(), NarratorError> {
    if plan.clips.is_empty() {
        return Err(NarratorError::ExportError("No clips to process".into()));
    }

    let meta = video_engine::probe_video(input_path).await?;
    let width = meta.width.max(2);
    let height = meta.height.max(2);
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
    let mut encoder =
        encoder::Encoder::start_with_aac(output_path, width, height, fps, audio_path.as_deref())
            .await?;

    let mut canvas = Pixmap::new(width, height)
        .ok_or_else(|| NarratorError::ExportError(format!("canvas alloc {width}x{height}")))?;
    let mut source_pix = Pixmap::new(width, height)
        .ok_or_else(|| NarratorError::ExportError(format!("source alloc {width}x{height}")))?;
    let mut last_decoded = vec![0u8; (width as usize) * (height as usize) * 4];
    let mut total_emitted: u64 = 0;

    for (clip_idx, clip) in plan.clips.iter().enumerate() {
        let clip_start = clip_starts[clip_idx];
        let out_dur = clip_output_duration(clip);
        let out_frames = (out_dur * fps).round().max(1.0) as u64;

        if clip.clip_type.as_deref() == Some("freeze") {
            // One source frame, repeated `out_frames` times. Per-clip
            // zoom-pan and overlay effects still animate over the duration.
            let frame_time = clip.freeze_source_time.unwrap_or(clip.start_seconds);
            let still =
                decoder::decode_single_frame_rgba(input_path, frame_time, width, height).await?;
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
                    on_progress(pct);
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
                input_path,
                clip.start_seconds,
                clip.end_seconds,
                width,
                height,
                decode_fps,
            )
            .await?;

            for f in 0..out_frames {
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
                    on_progress(pct);
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

    // Cleanup the temp audio.
    if let Some(p) = audio_path.as_ref() {
        let _ = tokio::fs::remove_file(p).await;
    }

    on_progress(100.0);
    Ok(())
}
