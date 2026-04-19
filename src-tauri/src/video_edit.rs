// Legacy ffmpeg `filter_complex` builders below (`build_effects_filter`,
// `build_progress_expr`, `generate_spotlight_mask`, etc.) were the second-pass
// effects pipeline. Phase 3 wired in the in-process compositor, so these are
// no longer reachable but remain in the tree until Phase 6 deletes them
// (their tests still exercise the helpers as a regression net while the
// compositor migration is in flight).
#![allow(dead_code)]
//! Video editing operations: trim, speed, frame dropping, zoom/pan, freeze frame, and concatenation.

use crate::error::NarratorError;
use crate::models::{EasingPreset, ZoomPanEffect};
use crate::process_utils::CommandNoWindow;
use crate::video_engine;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

// ── Validation helpers (S1–S5) ──

/// Validate a video path blocks obvious traversal attacks but allows user-selected files.
/// Files selected via native dialog can be anywhere on disk — we only reject paths that
/// try to escape via `..` components or point at system-critical directories.
fn validate_path(p: &str) -> Result<PathBuf, NarratorError> {
    let path = PathBuf::from(p);

    // Block raw ".." components (path traversal)
    for component in path.components() {
        if let std::path::Component::ParentDir = component {
            return Err(NarratorError::ExportError(format!(
                "Path contains '..': {p}"
            )));
        }
    }

    // Block system-critical paths (Unix)
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(canonical) = std::fs::canonicalize(&path) {
            let s = canonical.to_string_lossy();
            if s.starts_with("/etc")
                || s.starts_with("/bin")
                || s.starts_with("/sbin")
                || s.starts_with("/usr/bin")
                || s.starts_with("/usr/sbin")
                || s.starts_with("/System")
            {
                return Err(NarratorError::ExportError(format!("Path not allowed: {p}")));
            }
        }
    }

    Ok(path)
}

/// Validate clip parameters (S2: DoS, F4: bounds).
fn validate_clip(clip: &EditClip, duration: f64, index: usize) -> Result<(), NarratorError> {
    let err = |msg: &str| NarratorError::ExportError(format!("Clip {index}: {msg}"));
    // Reject NaN / Infinity — they produce invalid ffmpeg args and cause hangs or crashes.
    if !clip.speed.is_finite() || !clip.start_seconds.is_finite() || !clip.end_seconds.is_finite() {
        return Err(err("speed/start/end must be finite"));
    }
    if clip.speed <= 0.0 || clip.speed > 100.0 {
        return Err(err(&format!("speed {} out of range (0, 100]", clip.speed)));
    }
    if clip.start_seconds < -0.1 {
        return Err(err("start_seconds is negative"));
    }
    if clip.end_seconds < clip.start_seconds {
        return Err(err("end_seconds < start_seconds"));
    }
    // Reject zero/near-zero duration clips except freeze (which sets its own duration).
    let source_dur = clip.end_seconds - clip.start_seconds;
    let is_freeze = clip.clip_type.as_deref() == Some("freeze");
    if !is_freeze && source_dur < 0.05 {
        return Err(err(&format!(
            "clip duration {:.3}s too short (min 0.05s)",
            source_dur
        )));
    }
    if clip.end_seconds > duration + 5.0 {
        return Err(err(&format!(
            "end_seconds {:.1} exceeds video duration {:.1}",
            clip.end_seconds, duration
        )));
    }
    if let Some(fps) = clip.fps_override {
        if !fps.is_finite() || fps <= 0.0 || fps > 240.0 {
            return Err(err(&format!("fps_override {} out of range (0, 240]", fps)));
        }
    }
    if let Some(fd) = clip.freeze_duration {
        if !fd.is_finite() || fd <= 0.0 || fd > 600.0 {
            return Err(err(&format!(
                "freeze_duration {} out of range (0, 600]",
                fd
            )));
        }
    }
    if let Some(fst) = clip.freeze_source_time {
        if !fst.is_finite() || fst < 0.0 || fst > duration + 1.0 {
            return Err(err(&format!(
                "freeze_source_time {} out of video range",
                fst
            )));
        }
    }
    Ok(())
}

/// Validate zoom regions (S4: NaN/Infinity).
fn validate_zoom(zp: &ZoomPanEffect) -> Result<(), NarratorError> {
    let err = |field: &str| NarratorError::ExportError(format!("Zoom region has invalid {field}"));
    for (label, r) in [("start", &zp.start_region), ("end", &zp.end_region)] {
        if !r.x.is_finite() || !r.y.is_finite() || !r.width.is_finite() || !r.height.is_finite() {
            return Err(err(&format!("{label} region values")));
        }
        if r.width <= 0.0 || r.height <= 0.0 {
            return Err(err(&format!("{label} region size")));
        }
    }
    Ok(())
}

/// Escape text for ffmpeg drawtext filter (S3: injection prevention).
/// Will be used when text overlay rendering is implemented.
#[allow(dead_code)]
pub fn escape_ffmpeg_text(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\'', "'\\''")
        .replace(':', "\\:")
        .replace('%', "%%")
        .replace(['\n', '\r'], "")
}

/// Validate a hex color string (S3: injection prevention).
pub fn validate_hex_color(s: &str) -> Result<String, NarratorError> {
    let trimmed = s.trim().trim_start_matches('#');
    if (trimmed.len() == 6 || trimmed.len() == 8) && trimmed.chars().all(|c| c.is_ascii_hexdigit())
    {
        Ok(format!("#{trimmed}"))
    } else {
        Err(NarratorError::ExportError(format!(
            "Invalid hex color: {s}"
        )))
    }
}

/// Convert a hex color string (with or without `#`) to the ffmpeg `0xRRGGBB` form.
/// Falls back to `0x000000` for malformed input.
fn hex_to_ffmpeg_color(s: &str) -> String {
    match validate_hex_color(s) {
        Ok(c) => format!("0x{}", c.trim_start_matches('#')),
        Err(_) => "0x000000".to_string(),
    }
}

const MAX_OUTPUT_FPS: f64 = 60.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoEditPlan {
    pub clips: Vec<EditClip>,
    #[serde(default)]
    pub effects: Option<Vec<OverlayEffect>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayEffect {
    #[serde(rename = "type")]
    pub effect_type: String,
    pub start_time: f64,
    pub end_time: f64,
    #[serde(default)]
    pub transition_in: Option<f64>,
    #[serde(default)]
    pub transition_out: Option<f64>,
    #[serde(default)]
    pub reverse: Option<bool>,
    #[serde(default)]
    pub spotlight: Option<SpotlightData>,
    #[serde(default)]
    pub blur: Option<BlurData>,
    #[serde(default)]
    pub text: Option<TextData>,
    #[serde(default)]
    pub fade: Option<FadeData>,
    /// Present when effect_type == "zoom-pan". Carries the start/end regions
    /// and easing. Unlike the legacy per-clip zoom_pan on EditClip, this one
    /// is animated over its own [start_time, end_time] window.
    #[serde(default)]
    pub zoom_pan: Option<ZoomPanEffect>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpotlightData {
    pub x: f64,
    pub y: f64,
    pub radius: f64,
    pub dim_opacity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlurData {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub radius: f64,
    #[serde(default)]
    pub invert: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextData {
    pub content: String,
    pub x: f64,
    pub y: f64,
    pub font_size: f64,
    pub color: String,
    #[serde(default)]
    pub font_family: Option<String>,
    #[serde(default)]
    pub bold: Option<bool>,
    #[serde(default)]
    pub italic: Option<bool>,
    #[serde(default)]
    pub underline: Option<bool>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub align: Option<String>,
    #[serde(default)]
    pub opacity: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FadeData {
    pub color: String,
    pub opacity: f64,
}

/// Build an ffmpeg `filter_complex` expression that applies all overlay effects
/// in order, each chained via its own labeled intermediate stream.
///
/// Returns `Some((filter_complex_string, final_label))` or `None` if there are
/// no applicable effects. Coordinate convention: all positions are normalized
/// 0–1 and scaled to the output `width` × `height`.
/// Generate a PNG alpha mask with a transparent circular cutout on a
/// semi-transparent black background. Used to produce a visually clean
/// circular spotlight without relying on ffmpeg's `geq` filter (which is
/// O(pixels × frames × pow) and practically unusable on HD video).
///
/// The mask is cached by a content hash so repeat renders of the same
/// spotlight skip the write.
fn generate_spotlight_mask(
    width: u32,
    height: u32,
    cx: f64,
    cy: f64,
    radius: f64,
    dim_opacity: f64,
    out_dir: &Path,
) -> Result<PathBuf, NarratorError> {
    let dim_alpha = (dim_opacity.clamp(0.0, 1.0) * 255.0)
        .round()
        .clamp(0.0, 255.0) as u8;
    // Sub-pixel precision (3 decimals = 0.001 px) so two nearby spotlights
    // don't collide on the same cached mask. The earlier 1-decimal hash
    // could alias distinct effect positions that differ by less than 0.05 px.
    let hash = format!(
        "spot_{}x{}_{:.3}_{:.3}_{:.3}_{:03}",
        width, height, cx, cy, radius, dim_alpha
    );
    let path = out_dir.join(format!("{hash}.png"));
    if path.exists() {
        return Ok(path);
    }

    let mut img = image::RgbaImage::new(width, height);
    let r = radius.max(1.0);
    // 2-pixel anti-aliased band at the edge so the circle doesn't look
    // jagged. Wider = softer; 2 px is a good compromise.
    let feather: f64 = 2.0;
    let r_inner = (r - feather).max(0.0);
    for y in 0..height {
        for x in 0..width {
            let dx = x as f64 - cx;
            let dy = y as f64 - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            let alpha = if dist <= r_inner {
                0u8
            } else if dist >= r {
                dim_alpha
            } else {
                let t = (dist - r_inner) / feather;
                (dim_alpha as f64 * t).round().clamp(0.0, 255.0) as u8
            };
            img.put_pixel(x, y, image::Rgba([0, 0, 0, alpha]));
        }
    }
    img.save(&path)
        .map_err(|e| NarratorError::FfmpegFailed(format!("Failed to save spotlight mask: {e}")))?;
    Ok(path)
}

/// Build an ffmpeg expression for the effect's raw progress in [0, 1],
/// matching `effectProgress()` in `src/features/edit-video/easing.ts`.
///
/// The returned expression references `t` (current output time in seconds)
/// and evaluates to the animation progress for an effect that runs from
/// `s` to `s + dur`, with optional transition-in, transition-out, and
/// reverse (ramp up → hold → ramp down).
///
/// Every comma inside is pre-escaped with `\,` because this string is
/// substituted into filter-graph expressions where commas separate filter args.
fn build_progress_expr(s: f64, dur: f64, tin: f64, tout: f64, reverse: bool) -> String {
    let s3 = format!("{s:.3}");
    let d3 = format!("{dur:.3}");
    let local = format!("(t-{s3})");

    if reverse {
        // Three phases: ramp-in → hold → ramp-out.
        // Clamp tin + tout so they can't exceed the window.
        let tin_eff = tin.clamp(0.0, dur * 0.5);
        let tout_eff = tout.clamp(0.0, dur * 0.5);
        let hold_end = (dur - tout_eff).max(tin_eff);
        let he3 = format!("{hold_end:.3}");

        let ramp_in = if tin_eff > 0.001 {
            format!("{local}/{:.3}", tin_eff)
        } else {
            "1".to_string()
        };
        let ramp_out = if tout_eff > 0.001 {
            format!("({d3}-{local})/{:.3}", tout_eff)
        } else {
            "1".to_string()
        };
        // local < tin_eff ? ramp_in : (local < hold_end ? 1 : ramp_out)
        let inner = format!(
            "if(lt({local}\\,{:.3})\\,{ramp_in}\\,if(lt({local}\\,{he3})\\,1\\,{ramp_out}))",
            tin_eff
        );
        format!("max(0\\,min(1\\,{inner}))")
    } else if tin > 0.001 && tin < dur {
        // Ramp in over tin, then hold at 1 for the remainder.
        let ramp_in = format!("{local}/{:.3}", tin);
        let inner = format!("if(lt({local}\\,{:.3})\\,{ramp_in}\\,1)", tin);
        format!("max(0\\,min(1\\,{inner}))")
    } else {
        // No transition specified — animate over the full window.
        format!("max(0\\,min(1\\,{local}/{d3}))")
    }
}

fn build_effects_filter(
    effects: &[OverlayEffect],
    width: u32,
    height: u32,
    mask_dir: &Path,
) -> Option<(String, String, Vec<PathBuf>)> {
    let relevant: Vec<&OverlayEffect> = effects
        .iter()
        .filter(|e| {
            matches!(
                e.effect_type.as_str(),
                "spotlight" | "blur" | "text" | "fade" | "zoom-pan"
            )
        })
        .collect();
    if relevant.is_empty() {
        return None;
    }

    let w = width as f64;
    let h = height as f64;
    let max_wh = w.max(h);
    let mut parts: Vec<String> = Vec::new();
    let mut prev_label = "0:v".to_string();
    // Extra inputs for mask-based effects (e.g. spotlight). Their ffmpeg
    // input index is 1 + their position in this Vec (index 0 is the main
    // video). We track them here and return to the caller so it can add
    // them to the command line.
    let mut extra_inputs: Vec<PathBuf> = Vec::new();

    for (i, fx) in relevant.iter().enumerate() {
        let next_label = format!("fx{i}");
        let enable = format!(
            "between(t,{:.3},{:.3})",
            fx.start_time.max(0.0),
            fx.end_time.max(fx.start_time + 0.001)
        );

        match fx.effect_type.as_str() {
            "blur" => {
                let Some(b) = &fx.blur else {
                    continue;
                };
                // Clamp region to valid bounds
                let bx = (b.x.clamp(0.0, 1.0) * w).round() as i64;
                let by = (b.y.clamp(0.0, 1.0) * h).round() as i64;
                let bw_px = (b.width.clamp(0.01, 1.0) * w).round().max(2.0) as i64;
                let bh_px = (b.height.clamp(0.01, 1.0) * h).round().max(2.0) as i64;
                // Clamp blur radius: ffmpeg boxblur is slow for very large radii
                let radius = b.radius.clamp(1.0, 50.0);
                let invert = b.invert.unwrap_or(false);
                let split_a = format!("s{i}a");
                let split_b = format!("s{i}b");
                let blurred = format!("bl{i}");

                parts.push(format!("[{prev_label}]split=2[{split_a}][{split_b}]"));
                if invert {
                    // Blur everything; then paste the sharp crop back on top.
                    let crop = format!("cr{i}");
                    parts.push(format!(
                        "[{split_a}]boxblur=luma_radius={radius}:luma_power=1[{blurred}]"
                    ));
                    parts.push(format!("[{split_b}]crop={bw_px}:{bh_px}:{bx}:{by}[{crop}]"));
                    parts.push(format!(
                        "[{blurred}][{crop}]overlay={bx}:{by}:enable='{enable}'[{next_label}]"
                    ));
                } else {
                    // Crop the region, blur it, overlay back.
                    parts.push(format!(
                        "[{split_a}]crop={bw_px}:{bh_px}:{bx}:{by},boxblur=luma_radius={radius}:luma_power=1[{blurred}]"
                    ));
                    parts.push(format!(
                        "[{split_b}][{blurred}]overlay={bx}:{by}:enable='{enable}'[{next_label}]"
                    ));
                }
            }
            "spotlight" => {
                let Some(sp) = &fx.spotlight else {
                    continue;
                };
                // Circular spotlight via a pre-rendered alpha-mask PNG. The
                // mask is a black image with a transparent circle, saved once
                // and overlaid by ffmpeg on every frame during the effect's
                // time range. Far faster than geq (single precomputed image
                // vs. per-pixel/per-frame expression evaluation) and gives
                // true circular geometry.
                let cx_px = sp.x.clamp(0.0, 1.0) * w;
                let cy_px = sp.y.clamp(0.0, 1.0) * h;
                let r_px = (sp.radius.clamp(0.01, 1.0) * max_wh).max(4.0);
                let dim_opacity = sp.dim_opacity.clamp(0.0, 1.0);
                let mask_path = match generate_spotlight_mask(
                    width,
                    height,
                    cx_px,
                    cy_px,
                    r_px,
                    dim_opacity,
                    mask_dir,
                ) {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!("spotlight mask generation failed: {e}; skipping effect");
                        parts.push(format!("[{prev_label}]null[{next_label}]"));
                        prev_label = next_label;
                        continue;
                    }
                };
                // Input indices: [0] = main video, [1..] = each extra input
                // we've added so far.
                let mask_input_idx = 1 + extra_inputs.len();
                extra_inputs.push(mask_path);

                // Honor transitionIn/Out/reverse by pre-fading the mask's
                // alpha channel before overlaying. Previously these fields
                // were parsed but ignored — the export matched the UI only
                // for effects that had no transitions configured.
                //
                // Pattern from mature ffmpeg-driven editors: `fade=alpha=1`
                // multiplies the mask's alpha by a 0→1 (or 1→0) ramp using
                // the input stream's timestamp. The looped mask's stream
                // time aligns with the main video's time (both start at 0),
                // so `st=effect_start` fires at the right moment.
                let tin = fx.transition_in.unwrap_or(0.0).max(0.0);
                let tout = fx.transition_out.unwrap_or(0.0).max(0.0);
                let reverse = fx.reverse.unwrap_or(false);
                let fx_dur = fx.end_time.max(fx.start_time + 0.001) - fx.start_time;
                let s = fx.start_time.max(0.0);

                let faded_mask = format!("sm{i}");
                // Start by ensuring the mask carries an alpha channel fade
                // filter can touch. The generated PNG is already rgba, but
                // `format=rgba` is a cheap safety net.
                let mut mask_chain = format!("[{mask_input_idx}:v]format=rgba");

                if tin > 0.001 {
                    let tin_eff = tin.min(fx_dur * 0.5).max(0.001);
                    mask_chain.push_str(&format!(",fade=t=in:st={s:.3}:d={tin_eff:.3}:alpha=1"));
                }
                if reverse && tout > 0.001 {
                    let tout_eff = tout.min(fx_dur * 0.5).max(0.001);
                    let out_start = s + fx_dur - tout_eff;
                    mask_chain.push_str(&format!(
                        ",fade=t=out:st={out_start:.3}:d={tout_eff:.3}:alpha=1"
                    ));
                }
                mask_chain.push_str(&format!("[{faded_mask}]"));
                parts.push(mask_chain);
                parts.push(format!(
                    "[{prev_label}][{faded_mask}]overlay=0:0:enable='{enable}'[{next_label}]"
                ));
            }
            "text" => {
                let Some(t) = &fx.text else {
                    continue;
                };
                let text_esc = escape_ffmpeg_text(&t.content);
                if text_esc.trim().is_empty() {
                    // No text to draw — skip this effect but still chain label
                    parts.push(format!("[{prev_label}]null[{next_label}]"));
                    prev_label = next_label;
                    continue;
                }
                let fontsize_px = ((t.font_size.clamp(1.0, 40.0) / 100.0) * h)
                    .round()
                    .max(8.0);
                let color = hex_to_ffmpeg_color(&t.color);
                let opacity = t.opacity.unwrap_or(1.0).clamp(0.0, 1.0);
                let bold = t.bold.unwrap_or(false);
                let italic = t.italic.unwrap_or(false);
                // Center the text at the given normalized position
                let cx_px = (t.x.clamp(0.0, 1.0) * w).round();
                let cy_px = (t.y.clamp(0.0, 1.0) * h).round();
                let mut opts: Vec<String> = vec![
                    format!("text='{}'", text_esc),
                    format!("fontsize={}", fontsize_px),
                    format!("fontcolor={color}@{opacity:.3}"),
                    // Center anchor: position - text dimensions / 2
                    format!("x={:.0}-text_w/2", cx_px),
                    format!("y={:.0}-text_h/2", cy_px),
                    format!("enable='{enable}'"),
                ];
                // Bold/italic via libass-style font selection is not reliable in drawtext.
                // Emulate bold with borderw=2 (makes strokes thicker visually).
                if bold {
                    opts.push(format!("borderw=2:bordercolor={color}@{opacity:.3}"));
                }
                if italic {
                    // drawtext doesn't have italic; best-effort: skew-like offset via shadowx.
                    // Leaving as-is keeps font upright — UI already warns italic is preview-only.
                }
                if let Some(bg) = &t.background {
                    let bg_color = hex_to_ffmpeg_color(bg);
                    opts.push("box=1".into());
                    opts.push(format!("boxcolor={bg_color}@{opacity:.3}"));
                    opts.push("boxborderw=10".into());
                }
                let expr = format!("drawtext={}", opts.join(":"));
                parts.push(format!("[{prev_label}]{expr}[{next_label}]"));
            }
            "fade" => {
                let Some(f) = &fx.fade else {
                    continue;
                };
                let color = hex_to_ffmpeg_color(&f.color);
                let opacity = f.opacity.clamp(0.0, 1.0);
                // Full-frame color overlay with alpha
                let expr = format!(
                    "drawbox=x=0:y=0:w=iw:h=ih:color={color}@{opacity:.3}:t=fill:enable='{enable}'"
                );
                parts.push(format!("[{prev_label}]{expr}[{next_label}]"));
            }
            "zoom-pan" => {
                // Animated zoom/pan bound to the effect's time range. Pattern
                // borrowed from OpenShot's Timeline::apply_effects (see
                // libopenshot Timeline.cpp:554) — progress is LOCAL to the
                // effect, not stretched across the clip.
                let Some(zp) = &fx.zoom_pan else {
                    continue;
                };
                // Clamp regions defensively (same as per-clip builder)
                let sx = zp.start_region.x.clamp(0.0, 0.99);
                let sy = zp.start_region.y.clamp(0.0, 0.99);
                let sw = zp.start_region.width.clamp(0.05, 1.0);
                let sh = zp.start_region.height.clamp(0.05, 1.0);
                let ex = zp.end_region.x.clamp(0.0, 0.99);
                let ey = zp.end_region.y.clamp(0.0, 0.99);
                let ew = zp.end_region.width.clamp(0.05, 1.0);
                let eh = zp.end_region.height.clamp(0.05, 1.0);

                let s = fx.start_time.max(0.0);
                let e = fx.end_time.max(s + 0.01);
                let dur = (e - s).max(0.01);

                // Raw progress in [0, 1] matching `effectProgress()` in
                // `src/features/edit-video/easing.ts` so the export matches
                // the CSS preview. Honors `transitionIn`, `transitionOut`,
                // and `reverse` — previously these were silently dropped and
                // the export did a linear sweep regardless of the UI config.
                let tin = fx.transition_in.unwrap_or(0.0).max(0.0);
                let tout = fx.transition_out.unwrap_or(0.0).max(0.0);
                let reverse = fx.reverse.unwrap_or(false);
                let raw_p = build_progress_expr(s, dur, tin, tout, reverse);
                // Eased progress, matching the four UI easings.
                let eased = match zp.easing {
                    EasingPreset::Linear => raw_p.clone(),
                    EasingPreset::EaseIn => format!("({raw_p})*({raw_p})"),
                    EasingPreset::EaseOut => format!("({raw_p})*(2-({raw_p}))"),
                    EasingPreset::EaseInOut => format!(
                        "if(lt({raw_p}\\,0.5)\\,2*({raw_p})*({raw_p})\\,-1+(4-2*({raw_p}))*({raw_p}))"
                    ),
                };
                // Current region in normalized coords, as ffmpeg expressions.
                let cur_rw = format!("({sw:.4}+({ew:.4}-{sw:.4})*({eased}))");
                let cur_rh = format!("({sh:.4}+({eh:.4}-{sh:.4})*({eased}))");
                let cur_rx = format!("({sx:.4}+({ex:.4}-{sx:.4})*({eased}))");
                let cur_ry = format!("({sy:.4}+({ey:.4}-{sy:.4})*({eased}))");

                // Ken-Burns via time-varying scale + constant-size crop. The
                // previous approach used a crop whose output size varied per
                // frame, which triggered ffmpeg's "reinitializing filters"
                // path and failed on the second chained crop with
                // "Failed to configure input pad". With crop's W:H constant,
                // no re-config is needed downstream.
                //
                // Upscale factor = 1 / region_size(t): at full-frame regions
                // (rw=rh=1) the scale is identity; at small regions it's
                // large. Clamp to [1, 20] so we never downscale or produce
                // pathologically huge intermediates.
                let scale_w = format!("max({w}\\,min({w}*20\\,{w}/max(0.05\\,{cur_rw})))");
                let scale_h = format!("max({h}\\,min({h}*20\\,{h}/max(0.05\\,{cur_rh})))");
                // Crop position in the scaled image = region_origin * scaled_dim.
                // iw/ih inside crop = scale's output dims, so this resolves
                // per frame without any self-reference.
                let cx = format!("max(0\\,min(iw-{width}\\,{cur_rx}*iw))");
                let cy = format!("max(0\\,min(ih-{height}\\,{cur_ry}*ih))");

                let base = format!("zb{i}");
                let zoom_src = format!("zs{i}");
                let zoomed = format!("zd{i}");
                parts.push(format!("[{prev_label}]split=2[{base}][{zoom_src}]"));
                parts.push(format!(
                    "[{zoom_src}]scale=w='{scale_w}':h='{scale_h}':eval=frame:flags=lanczos,crop={width}:{height}:'{cx}':'{cy}',setsar=1[{zoomed}]"
                ));
                // Overlay only during the effect's time range; outside [s,e]
                // the un-zoomed base shows through.
                parts.push(format!(
                    "[{base}][{zoomed}]overlay=0:0:enable='{enable}'[{next_label}]"
                ));
            }
            _ => continue,
        }
        prev_label = next_label;
    }

    if parts.is_empty() {
        return None;
    }
    Some((parts.join(";"), prev_label, extra_inputs))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditClip {
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub speed: f64,
    #[serde(default)]
    pub skip_frames: bool,
    pub fps_override: Option<f64>,
    #[serde(default)]
    pub clip_type: Option<String>,
    #[serde(default)]
    pub freeze_source_time: Option<f64>,
    #[serde(default)]
    pub freeze_duration: Option<f64>,
    #[serde(default)]
    pub zoom_pan: Option<ZoomPanEffect>,
}

/// Run an ffmpeg command with real-time progress reporting.
/// Parses stderr for `time=` values and reports progress as 0.0-100.0.
async fn run_ffmpeg_with_progress(
    ffmpeg: &Path,
    args: &[&str],
    total_duration: f64,
    on_progress: &impl Fn(f64),
) -> Result<(), NarratorError> {
    // `-progress pipe:2` emits structured, newline-terminated progress events
    // (`out_time=HH:MM:SS.xxx`) to stderr so our line reader can parse them.
    // Default `frame=... time=...` stats use `\r` between updates, which
    // tokio's `lines()` doesn't split on — progress would never stream.
    // `-nostats` suppresses the default `\r`-terminated line. Both are global
    // options and must precede any output URL, so we prepend them.
    let mut full_args: Vec<&str> = vec!["-progress", "pipe:2", "-nostats"];
    full_args.extend_from_slice(args);
    let mut cmd = Command::new(ffmpeg.as_os_str());
    cmd.no_window()
        .args(&full_args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| NarratorError::FfmpegFailed(format!("Failed to start ffmpeg: {e}")))?;

    // Ring buffer of recent stderr lines so failures can include meaningful
    // context instead of just "exited with status 1". ffmpeg banners are
    // chatty; 40 lines is enough to catch the actual error tail.
    const STDERR_TAIL: usize = 40;
    let mut recent_stderr: std::collections::VecDeque<String> =
        std::collections::VecDeque::with_capacity(STDERR_TAIL + 1);

    if let Some(stderr) = child.stderr.take() {
        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(time_str) = extract_time_from_ffmpeg_line(&line) {
                let seconds = parse_ffmpeg_time(&time_str);
                if total_duration > 0.0 && seconds > 0.0 {
                    let pct = (seconds / total_duration * 100.0).min(100.0);
                    on_progress(pct);
                }
            }
            if recent_stderr.len() >= STDERR_TAIL {
                recent_stderr.pop_front();
            }
            recent_stderr.push_back(line);
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|e| NarratorError::FfmpegFailed(format!("ffmpeg process error: {e}")))?;

    if !status.success() {
        // Surface the most relevant stderr lines (those that look like errors)
        // falling back to the tail if nothing obvious stands out.
        let tail: Vec<&String> = recent_stderr.iter().collect();
        let meaningful: String = tail
            .iter()
            .filter(|l| {
                let ll = l.to_lowercase();
                ll.contains("error")
                    || ll.contains("invalid")
                    || ll.contains("no such")
                    || ll.contains("failed")
                    || ll.contains("unknown")
                    || ll.contains("unrecognized")
                    || ll.contains("does not")
            })
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        let detail = if meaningful.is_empty() {
            tail.iter()
                .rev()
                .take(5)
                .rev()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        } else {
            meaningful
        };
        return Err(NarratorError::FfmpegFailed(format!(
            "ffmpeg exited with status {status}: {detail}"
        )));
    }

    on_progress(100.0);
    Ok(())
}

/// Extract the time= value from an ffmpeg stderr line.
///
/// Handles both formats:
/// - Structured `-progress pipe:2` output: `out_time=00:00:01.666000\n`
///   (and `out_time_us=...`, `out_time_ms=...` — we prefer `out_time=`).
/// - Legacy `-stats` output: `frame=100 ... time=00:00:01.66 bitrate=...\r`.
fn extract_time_from_ffmpeg_line(line: &str) -> Option<String> {
    // Prefer the structured `-progress` format; it's \n-terminated so lines()
    // can actually see it.
    if let Some(i) = line.find("out_time=") {
        let rest = &line[i + 9..];
        let end = rest.find([' ', '\n', '\r']).unwrap_or(rest.len());
        let time_str = rest[..end].trim();
        if time_str.is_empty() || time_str == "N/A" {
            return None;
        }
        return Some(time_str.to_string());
    }
    // Fallback: legacy stats line. This only fires in contexts where we didn't
    // pass -nostats (we always do now, but keep the fallback for safety).
    let time_idx = line.find("time=")?;
    let rest = &line[time_idx + 5..];
    let end = rest.find(' ').unwrap_or(rest.len());
    let time_str = &rest[..end];
    if time_str == "N/A" {
        return None;
    }
    Some(time_str.to_string())
}

/// Parse ffmpeg time format "HH:MM:SS.ms" to seconds.
fn parse_ffmpeg_time(time_str: &str) -> f64 {
    let parts: Vec<&str> = time_str.split(':').collect();
    match parts.len() {
        3 => {
            let hours: f64 = parts[0].parse().unwrap_or(0.0);
            let minutes: f64 = parts[1].parse().unwrap_or(0.0);
            let seconds: f64 = parts[2].parse().unwrap_or(0.0);
            hours * 3600.0 + minutes * 60.0 + seconds
        }
        2 => {
            let minutes: f64 = parts[0].parse().unwrap_or(0.0);
            let seconds: f64 = parts[1].parse().unwrap_or(0.0);
            minutes * 60.0 + seconds
        }
        1 => parts[0].parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

/// Extract a single frame from a video at a given timestamp.
pub async fn extract_single_frame(
    video_path: &str,
    timestamp: f64,
    output_path: &str,
) -> Result<String, NarratorError> {
    let ffmpeg = video_engine::detect_ffmpeg()?;

    let output = Command::new(ffmpeg.as_os_str())
        .no_window()
        .args([
            "-y",
            "-ss",
            &format!("{:.3}", timestamp),
            "-i",
            video_path,
            "-frames:v",
            "1",
            "-q:v",
            "2",
            output_path,
        ])
        .output()
        .await
        .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(NarratorError::FfmpegFailed(format!(
            "Frame extraction failed: {}",
            &stderr[stderr.len().saturating_sub(500)..]
        )));
    }

    Ok(output_path.to_string())
}

/// Process a freeze frame clip: extract a single frame, then create a video of it held for the specified duration.
async fn process_freeze_clip(
    ffmpeg: &Path,
    input_path: &str,
    clip: &EditClip,
    clip_index: usize,
    out_dir: &Path,
    meta: &crate::models::VideoMetadata,
) -> Result<PathBuf, NarratorError> {
    let width = meta.width;
    let height = meta.height;
    let fps = meta.fps;
    let timestamp = clip.freeze_source_time.unwrap_or(clip.start_seconds);
    let duration = clip.freeze_duration.unwrap_or(3.0);
    let frame_path = out_dir.join(format!("_freeze_frame_{:03}.jpg", clip_index));
    let clip_path = out_dir.join(format!("_edit_clip_{:03}.mp4", clip_index));

    // Step 1: Extract the single frame
    let output = Command::new(ffmpeg.as_os_str())
        .no_window()
        .args([
            "-y",
            "-ss",
            &format!("{:.3}", timestamp),
            "-i",
            input_path,
            "-frames:v",
            "1",
            "-q:v",
            "2",
        ])
        .arg(frame_path.as_os_str())
        .output()
        .await
        .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(NarratorError::FfmpegFailed(format!(
            "Freeze frame extraction failed: {}",
            &stderr[stderr.len().saturating_sub(500)..]
        )));
    }

    // Step 2: Create a video from the still frame
    let mut args: Vec<String> = vec![
        "-y".into(),
        "-loop".into(),
        "1".into(),
        "-i".into(),
        frame_path.to_string_lossy().to_string(),
        "-t".into(),
        format!("{:.3}", duration),
    ];

    // If zoom/pan is specified, apply zoompan filter to the still image
    if let Some(ref zp) = clip.zoom_pan {
        let zp_filter = build_zoompan_filter(zp, width, height, fps, duration);
        args.extend([
            "-vf".into(),
            format!("{},scale={}:{}", zp_filter, width, height),
        ]);
    } else {
        args.extend(["-vf".into(), format!("scale={}:{}", width, height)]);
    }

    // Lossless h.264 encode (CRF 0). The freeze clip is a single JPG looped,
    // so file size stays small even at lossless.
    args.extend([
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "ultrafast".into(),
        "-crf".into(),
        "0".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-r".into(),
        format!("{:.0}", fps.min(MAX_OUTPUT_FPS)),
        "-an".into(), // no audio for freeze frames
    ]);
    args.push(clip_path.to_string_lossy().to_string());

    let output = Command::new(ffmpeg.as_os_str())
        .no_window()
        .args(&args)
        .output()
        .await
        .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(NarratorError::FfmpegFailed(format!(
            "Freeze clip creation failed: {}",
            &stderr[stderr.len().saturating_sub(500)..]
        )));
    }

    // Clean up extracted frame
    let _ = tokio::fs::remove_file(&frame_path).await;

    Ok(clip_path)
}

/// Build an ffmpeg zoompan filter expression from a ZoomPanEffect.
///
/// The zoompan filter uses per-frame expressions for z (zoom), x (pan-x), y (pan-y).
/// `on` = current frame number, `d` = total frames (set as the duration parameter).
fn build_zoompan_filter(
    effect: &ZoomPanEffect,
    width: u32,
    height: u32,
    fps: f64,
    duration_seconds: f64,
) -> String {
    let total_frames = (duration_seconds * fps.min(MAX_OUTPUT_FPS))
        .round()
        .max(1.0) as u32;

    // Zoom: 1/region_width gives the zoom factor (region covering 50% width = 2x zoom)
    let z_start = 1.0 / effect.start_region.width.max(0.01);
    let z_end = 1.0 / effect.end_region.width.max(0.01);

    // Pan centers (normalized 0-1)
    let sx = effect.start_region.x + effect.start_region.width / 2.0;
    let sy = effect.start_region.y + effect.start_region.height / 2.0;
    let ex = effect.end_region.x + effect.end_region.width / 2.0;
    let ey = effect.end_region.y + effect.end_region.height / 2.0;

    // Easing expression for progress (on/d mapped through easing function)
    let progress = match effect.easing {
        EasingPreset::Linear => "on/d".to_string(),
        EasingPreset::EaseIn => "(on/d)*(on/d)".to_string(),
        EasingPreset::EaseOut => "(on/d)*(2-on/d)".to_string(),
        EasingPreset::EaseInOut => {
            "if(lt(on/d,0.5),2*(on/d)*(on/d),-1+(4-2*(on/d))*(on/d))".to_string()
        }
    };

    format!(
        "zoompan=z='{z_s}+({z_e}-{z_s})*({p})':x='iw*({sx}+({ex}-{sx})*({p}))-iw/zoom/2':y='ih*({sy}+({ey}-{sy})*({p}))-ih/zoom/2':d={d}:s={w}x{h}:fps={fps}",
        z_s = z_start,
        z_e = z_end,
        p = progress,
        sx = sx,
        ex = ex,
        sy = sy,
        ey = ey,
        d = total_frames,
        w = width,
        h = height,
        fps = fps.min(MAX_OUTPUT_FPS).round() as u32,
    )
}

/// Build a crop+scale filter for zoom/pan on VIDEO clips (not stills).
/// Uses ffmpeg's `crop` filter with expression-based animated parameters
/// and `n` (frame number) for interpolation. This avoids the zoompan filter
/// issues with video input (zoompan is designed for still images).
/// Build a crop+scale filter for zoom/pan on VIDEO clips.
/// Uses ffmpeg's `crop` filter with expression-based animated parameters.
/// All values are clamped to valid ranges using min/max to prevent ffmpeg errors.
fn build_zoompan_filter_for_video(
    effect: &ZoomPanEffect,
    width: u32,
    height: u32,
    total_frames: f64,
) -> String {
    let w = width as f64;
    let h = height as f64;

    let sx = effect.start_region.x.clamp(0.0, 0.99);
    let sy = effect.start_region.y.clamp(0.0, 0.99);
    let sw = effect.start_region.width.clamp(0.05, 1.0);
    let sh = effect.start_region.height.clamp(0.05, 1.0);
    let ex = effect.end_region.x.clamp(0.0, 0.99);
    let ey = effect.end_region.y.clamp(0.0, 0.99);
    let ew = effect.end_region.width.clamp(0.05, 1.0);
    let eh = effect.end_region.height.clamp(0.05, 1.0);

    let tf = total_frames.max(1.0);

    let progress = match effect.easing {
        EasingPreset::Linear => format!("min(n/{tf},1)"),
        EasingPreset::EaseIn => format!("min(n/{tf},1)*min(n/{tf},1)"),
        EasingPreset::EaseOut => format!("min(n/{tf},1)*(2-min(n/{tf},1))"),
        EasingPreset::EaseInOut => format!(
            "if(lt(n/{tf},0.5),2*min(n/{tf},1)*min(n/{tf},1),-1+(4-2*min(n/{tf},1))*min(n/{tf},1))"
        ),
    };

    // Crop dimensions — clamped to at least 2px and at most iw/ih
    let crop_w = format!("max(2,min(iw,({sw}+({ew}-{sw})*({progress}))*{w}))");
    let crop_h = format!("max(2,min(ih,({sh}+({eh}-{sh})*({progress}))*{h}))");
    // Crop position — clamped so crop doesn't extend past frame edge
    let crop_x = format!("max(0,min(iw-out_w,({sx}+({ex}-{sx})*({progress}))*{w}))");
    let crop_y = format!("max(0,min(ih-out_h,({sy}+({ey}-{sy})*({progress}))*{h}))");

    // Ensure even dimensions for h264 compatibility
    format!(
        "crop='{crop_w}':'{crop_h}':'{crop_x}':'{crop_y}',scale={width}:{height}:flags=lanczos,setsar=1"
    )
}

pub async fn apply_edits(
    input_path: &str,
    output_path: &str,
    plan: &VideoEditPlan,
    on_progress: impl Fn(f64) + Send + Sync,
) -> Result<String, NarratorError> {
    // S1: Validate paths
    validate_path(input_path)?;
    validate_path(output_path)?;

    let ffmpeg = video_engine::detect_ffmpeg()?;
    let out_dir = Path::new(output_path).parent().unwrap_or(Path::new("/tmp"));
    let total = plan.clips.len();

    if total == 0 {
        return Err(NarratorError::ExportError("No clips to process".into()));
    }

    // Probe video metadata (needed for freeze frame and zoom/pan)
    let meta = video_engine::probe_video(std::path::Path::new(input_path)).await?;

    // Ensure the output directory exists. ffmpeg returns "No such file or
    // directory" when writing the MP4 trailer if the parent dir is missing —
    // it happens when the project dir hasn't been created yet (first-time
    // process before auto-save fires) or if the user renamed it.
    tokio::fs::create_dir_all(out_dir).await.map_err(|e| {
        NarratorError::ExportError(format!(
            "Failed to create output directory {}: {e}",
            out_dir.display()
        ))
    })?;

    // S2/F4: Validate all clips
    for (i, clip) in plan.clips.iter().enumerate() {
        validate_clip(clip, meta.duration_seconds, i)?;
        if let Some(ref zp) = clip.zoom_pan {
            validate_zoom(zp)?;
        }
    }

    // If single clip with no modifications, check if it covers the full source.
    // The fast path skips re-encoding entirely, so it can ONLY run when there
    // are no per-clip effects AND no overlay-track effects (blur/spotlight/
    // text/fade). Otherwise effects would be silently dropped.
    let has_clip_effects =
        plan.clips[0].clip_type.as_deref() == Some("freeze") || plan.clips[0].zoom_pan.is_some();
    let has_overlay_fx = plan
        .effects
        .as_ref()
        .map(|v| {
            v.iter().any(|e| {
                matches!(
                    e.effect_type.as_str(),
                    "spotlight" | "blur" | "text" | "fade"
                )
            })
        })
        .unwrap_or(false);
    let has_effects = has_clip_effects || has_overlay_fx;
    if total == 1
        && plan.clips[0].speed == 1.0
        && plan.clips[0].fps_override.is_none()
        && !has_effects
    {
        let clip = &plan.clips[0];

        // Check if the clip covers the full video (using already-probed metadata)
        let covers_full =
            clip.start_seconds < 0.5 && (clip.end_seconds - meta.duration_seconds).abs() < 0.5;

        if covers_full {
            // No edits — just use the original file directly (symlink or copy)
            if input_path != output_path {
                tokio::fs::copy(input_path, output_path).await?;
            }
            on_progress(100.0);
            return Ok(output_path.to_string());
        }

        // Trimmed single clip — use accurate seek (input seeking + output duration)
        let duration = clip.end_seconds - clip.start_seconds;
        let output = Command::new(ffmpeg.as_os_str())
            .no_window()
            .args([
                "-y",
                "-ss",
                &format!("{:.3}", clip.start_seconds),
                "-i",
                input_path,
                "-t",
                &format!("{:.3}", duration),
                "-c",
                "copy",
                "-avoid_negative_ts",
                "make_zero",
                output_path,
            ])
            .output()
            .await
            .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(NarratorError::FfmpegFailed(stderr.to_string()));
        }
        return Ok(output_path.to_string());
    }

    // Passthrough shortcut: if there's exactly one unmodified clip covering
    // the full source and the only work is overlay effects, skip the per-clip
    // re-encode entirely and feed `input_path` straight to the effects pass.
    // This was previously doing a full lossless re-encode just to produce a
    // concat target that the effects pass would read back seconds later —
    // roughly halves render time for the common case (single take + overlays).
    let covers_full_source = plan.clips[0].start_seconds < 0.5
        && (plan.clips[0].end_seconds - meta.duration_seconds).abs() < 0.5;
    let passthrough_to_fx = total == 1
        && plan.clips[0].speed == 1.0
        && plan.clips[0].fps_override.is_none()
        && !has_clip_effects
        && has_overlay_fx
        && covers_full_source;

    // Process each clip
    let mut clip_files: Vec<PathBuf> = Vec::new();

    // For smooth progress: give each clip a share of the 0-80% band
    // proportional to its expected output duration (so a 30s clip advances
    // the bar faster than a 10s clip at the same speed).
    let clip_weights: Vec<f64> = plan
        .clips
        .iter()
        .map(|c| {
            if c.clip_type.as_deref() == Some("freeze") {
                c.freeze_duration.unwrap_or(3.0).max(0.1)
            } else {
                let src = (c.end_seconds - c.start_seconds).max(0.01);
                if (c.speed - 1.0).abs() > 0.01 {
                    src / c.speed
                } else {
                    src
                }
            }
        })
        .collect();
    let total_weight: f64 = clip_weights.iter().sum::<f64>().max(0.01);

    // Skip the per-clip loop entirely in passthrough mode — the effects pass
    // will read `input_path` directly.
    for (i, clip) in plan.clips.iter().enumerate().filter(|_| !passthrough_to_fx) {
        // Base progress = sum of previous clips' shares (in 0-80% band).
        let cum_before: f64 = clip_weights.iter().take(i).sum::<f64>() / total_weight * 80.0;
        let this_share = clip_weights[i] / total_weight * 80.0;
        on_progress(cum_before);

        // Handle freeze frame clips separately
        if clip.clip_type.as_deref() == Some("freeze") {
            let clip_path =
                process_freeze_clip(&ffmpeg, input_path, clip, i, out_dir, &meta).await?;
            clip_files.push(clip_path);
            continue;
        }

        let clip_path = out_dir.join(format!("_edit_clip_{:03}.mp4", i));
        let clip_duration = clip.end_seconds - clip.start_seconds;
        let expected_output_dur = if (clip.speed - 1.0).abs() > 0.01 {
            clip_duration / clip.speed
        } else {
            clip_duration
        };

        // Belt-and-suspenders: input -t limits reading, output -t limits writing.
        // Input seeking (-ss before -i) resets PTS to 0 at the seek point.
        let mut args: Vec<String> = vec![
            "-y".into(),
            "-ss".into(),
            format!("{:.3}", clip.start_seconds),
            "-t".into(),
            format!("{:.3}", clip_duration), // INPUT -t: read exactly this many seconds
            "-i".into(),
            input_path.into(),
        ];

        // Build video filter chain
        let mut vfilters = Vec::new();
        let mut afilters = Vec::new();
        let needs_speed = (clip.speed - 1.0).abs() > 0.01;
        let has_zoom = clip.zoom_pan.is_some();

        // Zoom/Pan effect — animated crop+scale for video clips
        if let Some(ref zp) = clip.zoom_pan {
            let total_frames = (clip_duration * meta.fps.min(MAX_OUTPUT_FPS))
                .round()
                .max(1.0);
            let zp_filter =
                build_zoompan_filter_for_video(zp, meta.width, meta.height, total_frames);
            vfilters.push(zp_filter);
        }

        if let Some(fps) = clip.fps_override {
            vfilters.push(format!("fps={:.3}", fps));
        }

        if needs_speed {
            if clip.skip_frames {
                let n = clip.speed.round().max(2.0) as u32;
                vfilters.push(format!("select='not(mod(n\\,{}))'", n));
                vfilters.push("setpts=N/FRAME_RATE/TB".to_string());
            } else {
                vfilters.push(format!("setpts={:.4}*PTS", 1.0 / clip.speed));
            }
        }

        // Audio filter handling — atempo must be chained for values outside 0.5-2.0 range
        let drop_audio = needs_speed && clip.skip_frames;
        if needs_speed && !clip.skip_frames {
            let mut atempo_chain = Vec::new();
            let mut remaining = clip.speed;
            // Chain atempo=2.0 for speeds above 2.0
            while remaining > 2.0 {
                atempo_chain.push("atempo=2.0".to_string());
                remaining /= 2.0;
            }
            // Chain atempo=0.5 for speeds below 0.5
            while remaining < 0.5 {
                atempo_chain.push("atempo=0.5".to_string());
                remaining /= 0.5;
            }
            atempo_chain.push(format!("atempo={:.4}", remaining));
            afilters = atempo_chain;
        }

        // Always re-encode every clip with identical settings for reliable concat.
        // CRF 0 = lossless h.264 encoding. Since the source is yuv420p and we
        // keep yuv420p, this is bit-exact — no quality loss through the pipeline.
        // Files are larger than CRF 12 but exports preserve the original fidelity
        // the user recorded.
        vfilters.push("format=yuv420p".to_string());
        args.extend(["-vf".into(), vfilters.join(",")]);
        args.extend([
            "-c:v".into(),
            "libx264".into(),
            "-preset".into(),
            "ultrafast".into(),
            "-crf".into(),
            "0".into(),
            "-avoid_negative_ts".into(),
            "make_zero".into(),
        ]);

        // Limit output duration — critical for speed-changed clips where setpts
        // changes output timing but ffmpeg may read more input than needed.
        args.extend(["-t".into(), format!("{:.3}", expected_output_dur)]);
        if !afilters.is_empty() {
            args.extend(["-af".into(), afilters.join(",")]);
        }
        if !drop_audio {
            args.extend(["-c:a".into(), "aac".into(), "-b:a".into(), "256k".into()]);
        } else {
            args.extend(["-an".into()]);
        }
        args.extend(["-movflags".into(), "+faststart".into()]);
        args.push(clip_path.to_string_lossy().to_string());

        tracing::info!(
            "Clip {i}: src={:.3}-{:.3} ({:.3}s) speed={} expected_out={:.3}s zoom={has_zoom}",
            clip.start_seconds,
            clip.end_seconds,
            clip_duration,
            clip.speed,
            expected_output_dur
        );

        // Stream progress from ffmpeg stderr so the UI bar advances smoothly.
        // Each clip contributes `this_share` percent of the 0-80% band,
        // offset by `cum_before` (the sum of previous clips' shares).
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let progress_cb = |pct: f64| {
            on_progress(cum_before + (pct / 100.0) * this_share);
        };
        if let Err(e) =
            run_ffmpeg_with_progress(&ffmpeg, &arg_refs, expected_output_dur, &progress_cb).await
        {
            tracing::error!("Clip {i} ffmpeg args: {:?}", &args);
            return Err(NarratorError::FfmpegFailed(format!("Clip {i} failed: {e}")));
        }

        // Verify clip file was actually created and has content
        let clip_size = tokio::fs::metadata(&clip_path)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        // Log actual duration for debugging
        if let Ok(probe) = video_engine::probe_video(&clip_path).await {
            let drift = probe.duration_seconds - expected_output_dur;
            tracing::info!(
                "Clip {i}: actual_out={:.3}s (expected {:.3}s, drift={:.3}s)",
                probe.duration_seconds,
                expected_output_dur,
                drift
            );
        }
        if clip_size == 0 {
            tracing::error!("Clip {i} produced empty file: {}", clip_path.display());
            tracing::error!("Clip {i} ffmpeg args: {:?}", &args);
            return Err(NarratorError::FfmpegFailed(format!(
                "Clip {i} produced empty output. Try removing zoom/pan effects or simplifying edits on this clip."
            )));
        }
        tracing::info!("Clip {i} OK: {} bytes", clip_size);
        clip_files.push(clip_path);
    }

    on_progress(85.0);

    // If there are overlay effects, the concat writes to a temp file so we can
    // apply the effects filter graph in a second pass.
    let has_overlay_effects = plan
        .effects
        .as_ref()
        .map(|v| {
            v.iter().any(|e| {
                matches!(
                    e.effect_type.as_str(),
                    "spotlight" | "blur" | "text" | "fade"
                )
            })
        })
        .unwrap_or(false);

    let concat_target: String = if passthrough_to_fx {
        // Feed the original directly to the effects pass. `concat_target` is
        // not a temp file here, so the cleanup block below must NOT delete it.
        input_path.to_string()
    } else if has_overlay_effects {
        out_dir
            .join("_edit_concat_tmp.mp4")
            .to_string_lossy()
            .into_owned()
    } else {
        output_path.to_string()
    };

    // Concat all clips — all clips are re-encoded with identical h264 settings
    if passthrough_to_fx {
        // Nothing to concat; input feeds effects pass directly.
    } else if clip_files.len() == 1 {
        tokio::fs::rename(&clip_files[0], &concat_target).await?;
    } else {
        let concat_list = out_dir.join("_edit_concat.txt");
        let list_content: String = clip_files
            .iter()
            .map(|p| {
                let escaped = p
                    .to_string_lossy()
                    .replace('\\', "\\\\")
                    .replace(['\n', '\r'], "")
                    .replace('\'', "'\\''");
                format!("file '{}'", escaped)
            })
            .collect::<Vec<_>>()
            .join("\n");
        tokio::fs::write(&concat_list, &list_content).await?;

        tracing::info!("Concat: {} clips", clip_files.len());

        // All clips are identically encoded (h264/aac), so stream-copy concat should work
        let output = Command::new(ffmpeg.as_os_str())
            .no_window()
            .args(["-y", "-f", "concat", "-safe", "0", "-i"])
            .arg(concat_list.as_os_str())
            .args(["-c", "copy", "-movflags", "+faststart", &concat_target])
            .output()
            .await
            .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;

        if !output.status.success() {
            tracing::warn!("Stream-copy concat failed, falling back to re-encode");
            let output2 = Command::new(ffmpeg.as_os_str())
                .no_window()
                .args(["-y", "-f", "concat", "-safe", "0", "-i"])
                .arg(concat_list.as_os_str())
                .args([
                    "-c:v",
                    "libx264",
                    "-preset",
                    "ultrafast",
                    "-crf",
                    "0",
                    "-pix_fmt",
                    "yuv420p",
                    "-c:a",
                    "aac",
                    "-b:a",
                    "256k",
                    "-movflags",
                    "+faststart",
                    &concat_target,
                ])
                .output()
                .await
                .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;

            if !output2.status.success() {
                let stderr = String::from_utf8_lossy(&output2.stderr);
                return Err(NarratorError::FfmpegFailed(format!(
                    "Concat failed: {}",
                    &stderr[stderr.len().saturating_sub(500)..]
                )));
            }
        }

        let _ = tokio::fs::remove_file(&concat_list).await;
    }

    // Cleanup temp per-clip files
    for p in &clip_files {
        let _ = tokio::fs::remove_file(p).await;
    }

    // Apply overlay effects (spotlight, blur, text, fade, zoom-pan) in a
    // post-concat pass — Phase 3+ uses the in-process compositor instead of
    // a time-varying ffmpeg filter_complex. The compositor never builds a
    // varying graph, so the "Reinitializing filters" failure mode cannot
    // occur here by construction.
    if has_overlay_effects {
        let fx_start = if passthrough_to_fx { 0.0 } else { 90.0 };
        let fx_end = 99.0;
        on_progress(fx_start);

        let effects = plan.effects.as_deref().unwrap_or(&[]);
        let supported: Vec<crate::video_edit::OverlayEffect> = effects
            .iter()
            .filter(|e| {
                matches!(
                    e.effect_type.as_str(),
                    "spotlight" | "blur" | "text" | "fade" | "zoom-pan"
                )
            })
            .cloned()
            .collect();

        if !supported.is_empty() {
            tracing::info!(
                "Applying {} overlay effect(s) via in-process compositor (passthrough={})",
                supported.len(),
                passthrough_to_fx
            );

            let fx_progress_cb = |pct: f64| {
                let scaled = fx_start + (pct / 100.0) * (fx_end - fx_start);
                on_progress(scaled);
            };

            crate::compositor::apply_overlay_effects(
                std::path::Path::new(&concat_target),
                std::path::Path::new(output_path),
                &supported,
                &fx_progress_cb,
            )
            .await?;
        } else if !passthrough_to_fx {
            // No supported effects in `supported` — concat result is the output.
            tokio::fs::rename(&concat_target, output_path).await?;
        }
        // Clean up the intermediate concat file — but NEVER delete the user's
        // input in passthrough mode.
        if !passthrough_to_fx
            && std::path::Path::new(&concat_target).exists()
            && concat_target != output_path
        {
            let _ = tokio::fs::remove_file(&concat_target).await;
        }
    }

    on_progress(100.0);
    Ok(output_path.to_string())
}

pub async fn merge_audio_video(
    video_path: &str,
    audio_path: &str,
    output_path: &str,
    replace_audio: bool,
    on_progress: impl Fn(f64),
) -> Result<String, NarratorError> {
    let ffmpeg = video_engine::detect_ffmpeg()?;

    if replace_audio {
        // Replace original audio entirely with narration.
        // Uses -c:v copy so it's fast — no re-encoding needed.
        let output = Command::new(ffmpeg.as_os_str())
            .no_window()
            .args([
                "-y",
                "-i",
                video_path,
                "-i",
                audio_path,
                "-c:v",
                "copy",
                "-c:a",
                "aac",
                "-map",
                "0:v:0",
                "-map",
                "1:a:0",
                output_path,
            ])
            .output()
            .await
            .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(NarratorError::FfmpegFailed(format!(
                "Audio merge failed: {}",
                &stderr[stderr.len().saturating_sub(500)..]
            )));
        }

        on_progress(100.0);
        return Ok(output_path.to_string());
    }

    // Mix original + narration audio (amix) — re-encodes audio, so use progress reporting.
    let meta = video_engine::probe_video(Path::new(video_path)).await?;
    let total_duration = meta.duration_seconds;

    let filter = "[0:a][1:a]amix=inputs=2:duration=first:normalize=1[a]";
    let result = run_ffmpeg_with_progress(
        &ffmpeg,
        &[
            "-y",
            "-i",
            video_path,
            "-i",
            audio_path,
            "-filter_complex",
            filter,
            "-map",
            "0:v",
            "-map",
            "[a]",
            "-c:v",
            "copy",
            "-c:a",
            "aac",
            output_path,
        ],
        total_duration,
        &on_progress,
    )
    .await;

    if let Err(_e) = result {
        // Fallback: video might not have audio stream, use narration audio only
        tracing::warn!("amix failed, trying narration-only fallback");
        let fallback = Command::new(ffmpeg.as_os_str())
            .no_window()
            .args([
                "-y",
                "-i",
                video_path,
                "-i",
                audio_path,
                "-c:v",
                "copy",
                "-c:a",
                "aac",
                "-map",
                "0:v:0",
                "-map",
                "1:a:0",
                output_path,
            ])
            .output()
            .await
            .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;

        if !fallback.status.success() {
            let stderr = String::from_utf8_lossy(&fallback.stderr);
            return Err(NarratorError::FfmpegFailed(format!(
                "Audio merge failed: {}",
                &stderr[stderr.len().saturating_sub(500)..]
            )));
        }
    }

    on_progress(100.0);
    Ok(output_path.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleStyle {
    pub font_size: u32,
    pub color: String,
    pub outline_color: String,
    pub outline: u32,
    pub position: String,
}

impl Default for SubtitleStyle {
    fn default() -> Self {
        Self {
            font_size: 22,
            color: "#ffffff".to_string(),
            outline_color: "#000000".to_string(),
            outline: 2,
            position: "bottom".to_string(),
        }
    }
}

/// Convert a hex RGB color string (e.g. "#ffffff") to ffmpeg ASS BGR format (e.g. "&H00FFFFFF").
/// ASS colour format is &HAABBGGRR where AA=alpha (00=opaque).
fn hex_rgb_to_ass_bgr(hex: &str) -> String {
    let hex = hex.trim_start_matches('#');
    if hex.len() == 8 {
        // Has alpha channel (RRGGBBAA) — convert to ASS &HAABBGGRR
        let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(255);
        let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(255);
        let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(255);
        let a = u8::from_str_radix(&hex[6..8], 16).unwrap_or(0);
        // ASS alpha is inverted: 00 = opaque, FF = transparent
        let ass_alpha = 255 - a;
        format!("&H{:02X}{:02X}{:02X}{:02X}", ass_alpha, b, g, r)
    } else if hex.len() >= 6 {
        let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(255);
        let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(255);
        let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(255);
        format!("&H00{:02X}{:02X}{:02X}", b, g, r)
    } else {
        "&H00FFFFFF".to_string()
    }
}

pub async fn burn_subtitles(
    video_path: &str,
    srt_path: &str,
    output_path: &str,
    style: &SubtitleStyle,
    on_progress: impl Fn(f64),
) -> Result<String, NarratorError> {
    let ffmpeg = video_engine::detect_ffmpeg()?;

    // Probe video duration for progress reporting
    let meta = video_engine::probe_video(Path::new(video_path)).await?;
    let total_duration = meta.duration_seconds;

    // Copy SRT to temp dir with a unique name to avoid path escaping issues
    // with ffmpeg's subtitles filter (chokes on colons, spaces, special chars)
    let temp_srt =
        std::env::temp_dir().join(format!("_narrator_burn_subs_{}.srt", uuid::Uuid::new_v4()));
    tokio::fs::copy(srt_path, &temp_srt).await?;

    // Convert hex colors to ASS BGR format
    let primary_colour = hex_rgb_to_ass_bgr(&style.color);
    let outline_colour = hex_rgb_to_ass_bgr(&style.outline_color);

    // Position: bottom uses MarginV=30, top uses MarginV=10 + Alignment=6 (top-center)
    let position_style = if style.position == "top" {
        "MarginV=10,Alignment=6".to_string()
    } else {
        "MarginV=30".to_string()
    };

    // Try subtitles filter first (requires libass), fall back to SRT input method
    let srt_path_str = temp_srt
        .to_string_lossy()
        .replace('\\', "/")
        .replace(':', "\\:");
    // Sanitize numeric parameters to prevent unexpected ffmpeg filter behavior
    let font_size = style.font_size.clamp(8, 72);
    let outline = style.outline.clamp(0, 10);

    let subtitle_filter = format!(
        "subtitles='{}':force_style='FontSize={},PrimaryColour={},OutlineColour={},Outline={},BackColour=&H80000000,Shadow=1,{}'",
        srt_path_str, font_size, primary_colour, outline_colour, outline, position_style
    );

    let result = run_ffmpeg_with_progress(
        &ffmpeg,
        &[
            "-y",
            "-i",
            video_path,
            "-vf",
            &subtitle_filter,
            "-c:a",
            "copy",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "0",
            output_path,
        ],
        total_duration,
        &on_progress,
    )
    .await;

    if result.is_err() {
        // Fallback: use SRT as an input stream and overlay with mov_text → drawtext
        tracing::warn!("subtitles filter failed, trying SRT input overlay fallback");
        let fallback = Command::new(ffmpeg.as_os_str())
            .no_window()
            .args([
                "-y",
                "-i",
                video_path,
                "-i",
                &temp_srt.to_string_lossy(),
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-crf",
                "0",
                "-c:a",
                "copy",
                "-c:s",
                "mov_text",
                "-metadata:s:s:0",
                "language=eng",
                output_path,
            ])
            .output()
            .await
            .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;

        if !fallback.status.success() {
            let stderr = String::from_utf8_lossy(&fallback.stderr);
            let _ = tokio::fs::remove_file(&temp_srt).await;
            return Err(NarratorError::FfmpegFailed(format!(
                "Subtitle burn failed: {}",
                &stderr[..stderr.len().min(400)]
            )));
        }
    }

    let _ = tokio::fs::remove_file(&temp_srt).await;
    on_progress(100.0);

    Ok(output_path.to_string())
}

pub async fn extract_edit_thumbnails(
    video_path: &str,
    output_dir: &str,
    count: usize,
) -> Result<Vec<String>, NarratorError> {
    // Cache hit: if the output dir already has ≥ count JPGs AND the source
    // video hasn't been modified since they were produced, return them
    // without re-running ffmpeg. Checked BEFORE probing so repeat calls
    // (e.g. navigating back to Edit Video) are near-instant.
    {
        let dir = output_dir.to_string();
        let video = video_path.to_string();
        let cached = tokio::task::spawn_blocking(move || -> Option<Vec<String>> {
            let video_mtime = std::fs::metadata(&video).ok()?.modified().ok()?;
            let entries: Vec<std::fs::DirEntry> = std::fs::read_dir(&dir)
                .ok()?
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|x| x == "jpg"))
                .collect();
            if entries.len() < count {
                return None;
            }
            for entry in &entries {
                let t_meta = entry.metadata().ok()?;
                let t_mtime = t_meta.modified().ok()?;
                if t_mtime < video_mtime {
                    return None;
                }
            }
            let mut paths: Vec<String> = entries
                .into_iter()
                .map(|e| e.path().to_string_lossy().to_string())
                .collect();
            paths.sort();
            Some(paths)
        })
        .await
        .ok()
        .flatten();
        if let Some(paths) = cached {
            tracing::info!(
                "extract_edit_thumbnails: cache hit ({} thumbs in {})",
                paths.len(),
                output_dir
            );
            return Ok(paths);
        }
    }

    let ffmpeg = video_engine::detect_ffmpeg()?;
    let meta = video_engine::probe_video(Path::new(video_path)).await?;
    let interval = meta.duration_seconds / count as f64;

    tokio::fs::create_dir_all(output_dir).await?;

    let output = Command::new(ffmpeg.as_os_str())
        .no_window()
        .args([
            "-y",
            "-i",
            video_path,
            "-vf",
            &format!(
                "fps=1/{:.3},scale='min(120,iw)':'min(68,ih)':force_original_aspect_ratio=decrease",
                interval
            ),
            "-q:v",
            "5",
            &format!("{}/thumb_%04d.jpg", output_dir),
        ])
        .output()
        .await
        .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(NarratorError::FfmpegFailed(stderr.to_string()));
    }

    let dir = output_dir.to_string();
    let paths = tokio::task::spawn_blocking(move || {
        let mut paths: Vec<String> = std::fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "jpg"))
            .map(|e| e.path().to_string_lossy().to_string())
            .collect();
        paths.sort();
        Ok::<_, std::io::Error>(paths)
    })
    .await
    .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))??;
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_time_from_ffmpeg_line() {
        assert_eq!(
            extract_time_from_ffmpeg_line(
                "frame=  120 fps=30 q=28.0 size=    1024kB time=00:01:30.50 bitrate= 2094.1kbits/s"
            ),
            Some("00:01:30.50".to_string())
        );
        assert_eq!(extract_time_from_ffmpeg_line("time=N/A"), None);
        assert_eq!(extract_time_from_ffmpeg_line("no time here"), None);
    }

    #[test]
    fn test_parse_ffmpeg_time() {
        assert!((parse_ffmpeg_time("00:01:30.50") - 90.5).abs() < 0.01);
        assert!((parse_ffmpeg_time("01:00:00.00") - 3600.0).abs() < 0.01);
        assert!((parse_ffmpeg_time("00:00:05.25") - 5.25).abs() < 0.01);
        assert!((parse_ffmpeg_time("") - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_parse_ffmpeg_time_two_parts() {
        assert!((parse_ffmpeg_time("01:30.00") - 90.0).abs() < 0.01);
    }

    #[test]
    fn test_extract_time_at_end_of_line() {
        // time= at end of line with no trailing space
        assert_eq!(
            extract_time_from_ffmpeg_line("size=1024kB time=00:00:10.00"),
            Some("00:00:10.00".to_string())
        );
    }

    #[test]
    fn test_atempo_chaining_for_high_speeds() {
        // Simulate the atempo chain logic for speed=10x
        let speed = 10.0_f64;
        let mut chain = Vec::new();
        let mut remaining = speed;
        while remaining > 2.0 {
            chain.push("atempo=2.0".to_string());
            remaining /= 2.0;
        }
        while remaining < 0.5 {
            chain.push("atempo=0.5".to_string());
            remaining /= 0.5;
        }
        chain.push(format!("atempo={:.4}", remaining));

        // 10 / 2 / 2 / 2 = 1.25 → needs 3x atempo=2.0 + 1x atempo=1.25
        assert_eq!(chain.len(), 4);
        assert_eq!(chain[0], "atempo=2.0");
        assert_eq!(chain[1], "atempo=2.0");
        assert_eq!(chain[2], "atempo=2.0");
        assert!(chain[3].starts_with("atempo=1.25"));
        // Product should equal original speed: 2 * 2 * 2 * 1.25 = 10
        let product: f64 = chain
            .iter()
            .map(|s| s.strip_prefix("atempo=").unwrap().parse::<f64>().unwrap())
            .product();
        assert!((product - speed).abs() < 0.01);
    }

    #[test]
    fn test_expected_output_duration() {
        // 20 seconds at 10x speed = 2 seconds output
        let clip_duration: f64 = 20.0;
        let speed: f64 = 10.0;
        let expected = clip_duration / speed;
        assert!((expected - 2.0).abs() < 0.001);

        // 60 seconds at 1x speed = 60 seconds output
        let expected_1x: f64 = 60.0 / 1.0;
        assert!((expected_1x - 60.0).abs() < 0.001);

        // 30 seconds at 3x speed = 10 seconds output
        let expected_3x: f64 = 30.0 / 3.0;
        assert!((expected_3x - 10.0).abs() < 0.001);
    }

    #[test]
    fn test_validate_clip_bounds() {
        let clip = EditClip {
            start_seconds: 10.0,
            end_seconds: 30.0,
            speed: 5.0,
            skip_frames: false,
            fps_override: None,
            clip_type: None,
            freeze_source_time: None,
            freeze_duration: None,
            zoom_pan: None,
        };
        assert!(validate_clip(&clip, 60.0, 0).is_ok());

        // Speed 0 should fail
        let bad_speed = EditClip {
            speed: 0.0,
            ..clip.clone()
        };
        assert!(validate_clip(&bad_speed, 60.0, 0).is_err());

        // end < start should fail
        let bad_range = EditClip {
            start_seconds: 30.0,
            end_seconds: 10.0,
            ..clip.clone()
        };
        assert!(validate_clip(&bad_range, 60.0, 0).is_err());
    }

    #[test]
    fn test_validate_clip_rejects_nan_infinity() {
        let base = EditClip {
            start_seconds: 0.0,
            end_seconds: 10.0,
            speed: 1.0,
            skip_frames: false,
            fps_override: None,
            clip_type: None,
            freeze_source_time: None,
            freeze_duration: None,
            zoom_pan: None,
        };
        assert!(validate_clip(
            &EditClip {
                speed: f64::NAN,
                ..base.clone()
            },
            60.0,
            0
        )
        .is_err());
        assert!(validate_clip(
            &EditClip {
                speed: f64::INFINITY,
                ..base.clone()
            },
            60.0,
            0
        )
        .is_err());
        assert!(validate_clip(
            &EditClip {
                start_seconds: f64::NAN,
                ..base.clone()
            },
            60.0,
            0
        )
        .is_err());
        assert!(validate_clip(
            &EditClip {
                end_seconds: f64::NAN,
                ..base
            },
            60.0,
            0
        )
        .is_err());
    }

    #[test]
    fn test_validate_clip_rejects_zero_duration() {
        let clip = EditClip {
            start_seconds: 5.0,
            end_seconds: 5.0,
            speed: 1.0,
            skip_frames: false,
            fps_override: None,
            clip_type: None,
            freeze_source_time: None,
            freeze_duration: None,
            zoom_pan: None,
        };
        assert!(validate_clip(&clip, 60.0, 0).is_err());
    }

    #[test]
    fn test_validate_clip_allows_freeze_zero_source() {
        // Freeze clip source span can be zero — duration comes from freeze_duration
        let clip = EditClip {
            start_seconds: 5.0,
            end_seconds: 5.0,
            speed: 1.0,
            skip_frames: false,
            fps_override: None,
            clip_type: Some("freeze".into()),
            freeze_source_time: Some(5.0),
            freeze_duration: Some(3.0),
            zoom_pan: None,
        };
        assert!(validate_clip(&clip, 60.0, 0).is_ok());
    }

    #[test]
    fn test_validate_clip_rejects_bad_fps_override() {
        let base = EditClip {
            start_seconds: 0.0,
            end_seconds: 10.0,
            speed: 1.0,
            skip_frames: false,
            fps_override: Some(1000.0),
            clip_type: None,
            freeze_source_time: None,
            freeze_duration: None,
            zoom_pan: None,
        };
        assert!(validate_clip(&base, 60.0, 0).is_err());

        let nan = EditClip {
            fps_override: Some(f64::NAN),
            ..base.clone()
        };
        assert!(validate_clip(&nan, 60.0, 0).is_err());
    }

    #[test]
    fn test_validate_clip_rejects_bad_freeze_source_time() {
        let clip = EditClip {
            start_seconds: 0.0,
            end_seconds: 2.0,
            speed: 1.0,
            skip_frames: false,
            fps_override: None,
            clip_type: Some("freeze".into()),
            freeze_source_time: Some(999.0),
            freeze_duration: Some(3.0),
            zoom_pan: None,
        };
        assert!(validate_clip(&clip, 60.0, 0).is_err());
    }

    #[test]
    fn test_validate_zoom_rejects_nan() {
        let bad = ZoomPanEffect {
            start_region: crate::models::ZoomRegion {
                x: f64::NAN,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            end_region: crate::models::ZoomRegion {
                x: 0.0,
                y: 0.0,
                width: 0.5,
                height: 0.5,
            },
            easing: EasingPreset::Linear,
        };
        assert!(validate_zoom(&bad).is_err());
    }

    #[test]
    fn test_validate_zoom_rejects_zero_size() {
        let bad = ZoomPanEffect {
            start_region: crate::models::ZoomRegion {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 1.0,
            },
            end_region: crate::models::ZoomRegion {
                x: 0.0,
                y: 0.0,
                width: 0.5,
                height: 0.5,
            },
            easing: EasingPreset::Linear,
        };
        assert!(validate_zoom(&bad).is_err());
    }

    #[test]
    fn test_build_effects_filter_handles_inverted_time() {
        // end < start → our code clamps with max() so it doesn't emit a broken enable expr
        let mut fx = empty_effect("fade");
        fx.start_time = 10.0;
        fx.end_time = 5.0;
        fx.fade = Some(FadeData {
            color: "#000000".into(),
            opacity: 0.5,
        });
        // Should not panic; filter should contain a valid between()
        let result = run_effects_filter(&[fx], 320, 240);
        assert!(result.is_some());
        let (f, _, _) = result.unwrap();
        assert!(f.contains("between(t,"));
    }

    #[test]
    fn test_build_effects_filter_handles_degenerate_blur_region() {
        // 0-size blur region should clamp to a tiny valid region rather than fail
        let mut fx = empty_effect("blur");
        fx.blur = Some(BlurData {
            x: 0.5,
            y: 0.5,
            width: 0.0,
            height: 0.0,
            radius: 10.0,
            invert: None,
        });
        let result = run_effects_filter(&[fx], 320, 240);
        assert!(result.is_some());
    }

    #[test]
    fn test_build_effects_filter_text_empty_content_skipped_cleanly() {
        let mut fx = empty_effect("text");
        fx.text = Some(TextData {
            content: "   ".into(),
            x: 0.5,
            y: 0.5,
            font_size: 5.0,
            color: "#ffffff".into(),
            font_family: None,
            bold: None,
            italic: None,
            underline: None,
            background: None,
            align: None,
            opacity: None,
        });
        // Empty content → null filter that preserves the stream label
        let (f, _, _) = run_effects_filter(&[fx], 320, 240).unwrap();
        assert!(f.contains("null"));
    }

    #[test]
    fn test_build_effects_filter_text_unicode_ok() {
        let mut fx = empty_effect("text");
        fx.text = Some(TextData {
            content: "日本語のテキスト 🎬".into(),
            x: 0.5,
            y: 0.5,
            font_size: 5.0,
            color: "#ffffff".into(),
            font_family: None,
            bold: None,
            italic: None,
            underline: None,
            background: None,
            align: None,
            opacity: None,
        });
        let (f, _, _) = run_effects_filter(&[fx], 320, 240).unwrap();
        // Unicode content should pass through (drawtext supports UTF-8)
        assert!(f.contains("日本語のテキスト 🎬"));
    }

    // ── build_effects_filter ──────────────────────────────────────

    fn empty_effect(kind: &str) -> OverlayEffect {
        OverlayEffect {
            effect_type: kind.to_string(),
            start_time: 0.0,
            end_time: 5.0,
            transition_in: None,
            transition_out: None,
            reverse: None,
            spotlight: None,
            blur: None,
            text: None,
            fade: None,
            zoom_pan: None,
        }
    }

    /// Helper: call build_effects_filter with a fresh temp mask dir so tests
    /// don't pollute each other's cache.
    fn run_effects_filter(
        effects: &[OverlayEffect],
        w: u32,
        h: u32,
    ) -> Option<(String, String, Vec<PathBuf>)> {
        let dir = tempfile::tempdir().unwrap();
        build_effects_filter(effects, w, h, dir.path())
    }

    #[test]
    fn test_build_effects_filter_empty() {
        assert!(run_effects_filter(&[], 1920, 1080).is_none());
    }

    #[test]
    fn test_build_effects_filter_zoom_pan_ignored() {
        // zoom-pan is handled per-clip, not in the post-pass
        let fx = empty_effect("zoom-pan");
        assert!(run_effects_filter(&[fx], 1920, 1080).is_none());
    }

    #[test]
    fn test_build_effects_filter_blur() {
        let mut fx = empty_effect("blur");
        fx.blur = Some(BlurData {
            x: 0.25,
            y: 0.25,
            width: 0.5,
            height: 0.5,
            radius: 20.0,
            invert: None,
        });
        let (f, final_label, _) = run_effects_filter(&[fx], 1920, 1080).unwrap();
        assert!(f.contains("boxblur"));
        assert!(f.contains("crop"));
        assert!(f.contains("overlay"));
        assert!(f.contains("between(t,0.000,5.000)"));
        assert_eq!(final_label, "fx0");
    }

    #[test]
    fn test_build_effects_filter_blur_invert() {
        let mut fx = empty_effect("blur");
        fx.blur = Some(BlurData {
            x: 0.1,
            y: 0.1,
            width: 0.2,
            height: 0.2,
            radius: 10.0,
            invert: Some(true),
        });
        let (f, _, _) = run_effects_filter(&[fx], 1920, 1080).unwrap();
        // Invert mode: boxblur runs on full frame (no crop before it)
        assert!(f.contains("boxblur"));
        assert!(f.contains("crop"));
    }

    #[test]
    fn test_build_effects_filter_spotlight() {
        let mut fx = empty_effect("spotlight");
        fx.spotlight = Some(SpotlightData {
            x: 0.5,
            y: 0.5,
            radius: 0.2,
            dim_opacity: 0.6,
        });
        // Keep the tempdir alive for the whole test so the written mask
        // file doesn't get cleaned up before we check it.
        let dir = tempfile::tempdir().unwrap();
        let (f, _, extras) = build_effects_filter(&[fx], 1920, 1080, dir.path()).unwrap();
        assert_eq!(extras.len(), 1, "spotlight should add 1 extra PNG input");
        // Mask is now pre-chained through format=rgba (and optionally fade
        // filters when the effect has transitions) into a named label that
        // overlay consumes — verify the overlay picks up the processed mask.
        assert!(
            f.contains("[1:v]format=rgba"),
            "filter should normalize mask to rgba: {f}"
        );
        assert!(
            f.contains("][sm0]overlay=0:0"),
            "overlay should consume the normalized mask label: {f}"
        );
        assert!(f.contains("between(t,0.000,5.000)"));
        assert!(extras[0].exists(), "mask PNG should be written");
    }

    #[test]
    fn test_build_effects_filter_text() {
        let mut fx = empty_effect("text");
        fx.text = Some(TextData {
            content: "Hello".into(),
            x: 0.5,
            y: 0.1,
            font_size: 5.0,
            color: "#ffffff".into(),
            font_family: None,
            bold: Some(true),
            italic: None,
            underline: None,
            background: None,
            align: None,
            opacity: Some(0.9),
        });
        let (f, _, _) = run_effects_filter(&[fx], 1920, 1080).unwrap();
        assert!(f.contains("drawtext"));
        assert!(f.contains("Hello"));
        assert!(f.contains("text_w/2"));
        assert!(f.contains("borderw=2")); // bold emulation
    }

    #[test]
    fn test_build_effects_filter_text_escapes_special_chars() {
        let mut fx = empty_effect("text");
        fx.text = Some(TextData {
            content: "Hello: 100%'s".into(),
            x: 0.5,
            y: 0.5,
            font_size: 5.0,
            color: "#ffffff".into(),
            font_family: None,
            bold: None,
            italic: None,
            underline: None,
            background: None,
            align: None,
            opacity: None,
        });
        let (f, _, _) = run_effects_filter(&[fx], 1920, 1080).unwrap();
        // Colons must be escaped so drawtext parses correctly
        assert!(f.contains("Hello\\:"));
        assert!(f.contains("100%%"));
    }

    #[test]
    fn test_build_effects_filter_fade() {
        let mut fx = empty_effect("fade");
        fx.start_time = 2.0;
        fx.end_time = 3.5;
        fx.fade = Some(FadeData {
            color: "#000000".into(),
            opacity: 0.5,
        });
        let (f, _, _) = run_effects_filter(&[fx], 1920, 1080).unwrap();
        assert!(f.contains("drawbox"));
        assert!(f.contains("0x000000@0.500"));
        assert!(f.contains("between(t,2.000,3.500)"));
    }

    #[test]
    fn test_build_effects_filter_chains_multiple() {
        let mut f1 = empty_effect("blur");
        f1.blur = Some(BlurData {
            x: 0.0,
            y: 0.0,
            width: 0.5,
            height: 0.5,
            radius: 10.0,
            invert: None,
        });
        let mut f2 = empty_effect("fade");
        f2.fade = Some(FadeData {
            color: "#ffffff".into(),
            opacity: 0.3,
        });
        let (filter, final_label, _) = run_effects_filter(&[f1, f2], 1920, 1080).unwrap();
        // Last effect's output label should be fx1
        assert_eq!(final_label, "fx1");
        // Both effects should appear
        assert!(filter.contains("boxblur"));
        assert!(filter.contains("drawbox"));
    }

    // ── zoom-pan as a timeline effect (new) ─────────────────────────────

    #[test]
    fn test_build_effects_filter_zoom_pan_branch() {
        let mut fx = empty_effect("zoom-pan");
        fx.start_time = 10.0;
        fx.end_time = 14.0;
        fx.zoom_pan = Some(ZoomPanEffect {
            start_region: crate::models::ZoomRegion {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            end_region: crate::models::ZoomRegion {
                x: 0.25,
                y: 0.25,
                width: 0.5,
                height: 0.5,
            },
            easing: EasingPreset::Linear,
        });
        let (f, _, _) = run_effects_filter(&[fx], 1920, 1080).unwrap();
        // Ken Burns via time-varying scale + constant-size crop (avoids the
        // ffmpeg "reinitializing filters" path that killed the previous
        // time-varying crop approach). Crop's W:H are now literal constants.
        assert!(f.contains("scale=w='"), "expected time-varying scale: {f}");
        assert!(
            f.contains("eval=frame"),
            "scale must re-evaluate per frame: {f}"
        );
        assert!(
            f.contains("crop=1920:1080:"),
            "expected constant-size crop: {f}"
        );
        // Time range should be embedded in the overlay enable expression.
        assert!(
            f.contains("between(t,10.000,14.000)"),
            "expected effect-local time bounds: {f}"
        );
        // Raw progress formula uses the effect's start+duration, NOT total clip.
        assert!(
            f.contains("(t-10.000)/4.000"),
            "expected effect-local progress formula: {f}"
        );
    }

    #[test]
    fn test_build_effects_filter_zoom_pan_easing_ease_in() {
        let mut fx = empty_effect("zoom-pan");
        fx.start_time = 5.0;
        fx.end_time = 10.0;
        fx.zoom_pan = Some(ZoomPanEffect {
            start_region: crate::models::ZoomRegion {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            end_region: crate::models::ZoomRegion {
                x: 0.1,
                y: 0.1,
                width: 0.6,
                height: 0.6,
            },
            easing: EasingPreset::EaseIn,
        });
        let (f, _, _) = run_effects_filter(&[fx], 1920, 1080).unwrap();
        // Ease-in: p*p. The squared progress should appear as (raw)*(raw).
        assert!(
            f.contains(")*(max(0"),
            "expected p*p pattern for ease-in: {f}"
        );
    }

    #[test]
    fn test_build_effects_filter_two_zoom_pans_both_applied() {
        // Two zoom effects at different times: both must appear (prior bug
        // only attached the first to the clip).
        let mut f1 = empty_effect("zoom-pan");
        f1.start_time = 2.0;
        f1.end_time = 5.0;
        f1.zoom_pan = Some(ZoomPanEffect {
            start_region: crate::models::ZoomRegion {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            end_region: crate::models::ZoomRegion {
                x: 0.2,
                y: 0.2,
                width: 0.5,
                height: 0.5,
            },
            easing: EasingPreset::Linear,
        });
        let mut f2 = empty_effect("zoom-pan");
        f2.start_time = 10.0;
        f2.end_time = 13.0;
        f2.zoom_pan = Some(ZoomPanEffect {
            start_region: crate::models::ZoomRegion {
                x: 0.5,
                y: 0.5,
                width: 0.4,
                height: 0.4,
            },
            end_region: crate::models::ZoomRegion {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            easing: EasingPreset::EaseOut,
        });
        let (filter, final_label, _) = run_effects_filter(&[f1, f2], 1920, 1080).unwrap();
        assert_eq!(final_label, "fx1");
        // Both time ranges present
        assert!(filter.contains("between(t,2.000,5.000)"));
        assert!(filter.contains("between(t,10.000,13.000)"));
    }

    #[test]
    fn test_hex_to_ffmpeg_color() {
        assert_eq!(hex_to_ffmpeg_color("#ff00aa"), "0xff00aa");
        assert_eq!(hex_to_ffmpeg_color("ff00aa"), "0xff00aa");
        // Invalid → black fallback
        assert_eq!(hex_to_ffmpeg_color("not-a-color"), "0x000000");
    }

    // ── Integration tests (run real ffmpeg) ────────────────────────
    //
    // These tests create a short test video with `lavfi testsrc` and an audio
    // sine tone, then run the real editing pipeline. They take a few seconds
    // each and require ffmpeg on PATH. If ffmpeg isn't available the test is
    // reported as skipped (early return).

    fn ffmpeg_ok() -> bool {
        video_engine::detect_ffmpeg().is_ok()
    }

    /// Some ffmpeg builds (notably Homebrew's default) ship without libfreetype,
    /// so the `drawtext` filter isn't registered. Tests that exercise text
    /// overlays skip themselves when this is the case.
    fn drawtext_available() -> bool {
        let Ok(ffmpeg) = video_engine::detect_ffmpeg() else {
            return false;
        };
        let Ok(output) = std::process::Command::new(ffmpeg).arg("-filters").output() else {
            return false;
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout.contains("drawtext")
    }

    async fn make_test_video(path: &Path, duration_s: f64) -> Result<(), String> {
        let ffmpeg = video_engine::detect_ffmpeg().map_err(|e| e.to_string())?;
        let output = Command::new(&ffmpeg)
            .no_window()
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                &format!("testsrc=duration={duration_s}:size=320x240:rate=30"),
                "-f",
                "lavfi",
                "-i",
                &format!("sine=frequency=1000:duration={duration_s}"),
                "-c:v",
                "libx264",
                // Force keyframe every 15 frames (0.5s) so trims are accurate when
                // the fast-path stream-copy seeks to the nearest keyframe.
                "-g",
                "15",
                "-keyint_min",
                "15",
                "-c:a",
                "aac",
                "-pix_fmt",
                "yuv420p",
                "-shortest",
            ])
            .arg(path)
            .output()
            .await
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(format!("make_test_video failed: {err}"));
        }
        Ok(())
    }

    fn simple_clip(start: f64, end: f64, speed: f64) -> EditClip {
        EditClip {
            start_seconds: start,
            end_seconds: end,
            speed,
            skip_frames: false,
            fps_override: None,
            clip_type: None,
            freeze_source_time: None,
            freeze_duration: None,
            zoom_pan: None,
        }
    }

    fn base_effect(kind: &str, start: f64, end: f64) -> OverlayEffect {
        OverlayEffect {
            effect_type: kind.to_string(),
            start_time: start,
            end_time: end,
            transition_in: None,
            transition_out: None,
            reverse: None,
            spotlight: None,
            blur: None,
            text: None,
            fade: None,
            zoom_pan: None,
        }
    }

    async fn apply_and_probe(
        plan: VideoEditPlan,
        input_duration: f64,
    ) -> Result<(f64, u32, u32), String> {
        let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
        let input = dir.path().join("input.mp4");
        let output = dir.path().join("output.mp4");
        make_test_video(&input, input_duration).await?;
        apply_edits(
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            &plan,
            |_| {},
        )
        .await
        .map_err(|e| e.to_string())?;
        let meta = video_engine::probe_video(&output)
            .await
            .map_err(|e| e.to_string())?;
        Ok((meta.duration_seconds, meta.width, meta.height))
    }

    #[tokio::test]
    async fn integration_trim_preserves_duration() {
        if !ffmpeg_ok() {
            return;
        }
        let plan = VideoEditPlan {
            clips: vec![simple_clip(2.0, 6.0, 1.0)],
            effects: None,
        };
        let (dur, w, h) = apply_and_probe(plan, 10.0).await.unwrap();
        assert!((dur - 4.0).abs() < 0.3, "expected ~4s, got {dur}s");
        assert_eq!((w, h), (320, 240));
    }

    #[tokio::test]
    async fn integration_speed_2x_halves_duration() {
        if !ffmpeg_ok() {
            return;
        }
        // 6s of source at 2x should produce a 3s output
        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 6.0, 2.0)],
            effects: None,
        };
        let (dur, _, _) = apply_and_probe(plan, 10.0).await.unwrap();
        assert!(
            (dur - 3.0).abs() < 0.3,
            "expected ~3s at 2x speed, got {dur}s"
        );
    }

    #[tokio::test]
    async fn integration_concat_multiple_clips() {
        if !ffmpeg_ok() {
            return;
        }
        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 2.0, 1.0), simple_clip(4.0, 6.0, 1.0)],
            effects: None,
        };
        let (dur, _, _) = apply_and_probe(plan, 10.0).await.unwrap();
        assert!((dur - 4.0).abs() < 0.5, "expected ~4s concat, got {dur}s");
    }

    #[tokio::test]
    async fn integration_zoom_pan() {
        if !ffmpeg_ok() {
            return;
        }
        let mut clip = simple_clip(0.0, 4.0, 1.0);
        clip.zoom_pan = Some(ZoomPanEffect {
            start_region: crate::models::ZoomRegion {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            end_region: crate::models::ZoomRegion {
                x: 0.25,
                y: 0.25,
                width: 0.5,
                height: 0.5,
            },
            easing: EasingPreset::EaseInOut,
        });
        let plan = VideoEditPlan {
            clips: vec![clip],
            effects: None,
        };
        let (dur, _, _) = apply_and_probe(plan, 10.0).await.unwrap();
        assert!(dur > 3.5 && dur < 4.5, "expected ~4s with zoom, got {dur}s");
    }

    #[tokio::test]
    async fn integration_blur_effect() {
        if !ffmpeg_ok() {
            return;
        }
        let mut fx = base_effect("blur", 0.5, 2.5);
        fx.blur = Some(BlurData {
            x: 0.25,
            y: 0.25,
            width: 0.5,
            height: 0.5,
            radius: 15.0,
            invert: None,
        });
        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 3.0, 1.0)],
            effects: Some(vec![fx]),
        };
        let (dur, _, _) = apply_and_probe(plan, 10.0).await.unwrap();
        assert!(dur > 2.7 && dur < 3.3, "expected ~3s with blur, got {dur}s");
    }

    #[tokio::test]
    async fn integration_spotlight_effect() {
        if !ffmpeg_ok() {
            return;
        }
        let mut fx = base_effect("spotlight", 0.0, 2.0);
        fx.spotlight = Some(SpotlightData {
            x: 0.5,
            y: 0.5,
            radius: 0.25,
            dim_opacity: 0.6,
        });
        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 3.0, 1.0)],
            effects: Some(vec![fx]),
        };
        let (dur, _, _) = apply_and_probe(plan, 10.0).await.unwrap();
        assert!(
            dur > 2.7 && dur < 3.3,
            "expected ~3s with spotlight, got {dur}s"
        );
    }

    #[tokio::test]
    async fn integration_text_effect() {
        if !ffmpeg_ok() || !drawtext_available() {
            return;
        }
        let mut fx = base_effect("text", 0.5, 2.5);
        fx.text = Some(TextData {
            content: "Test Overlay".into(),
            x: 0.5,
            y: 0.1,
            font_size: 5.0,
            color: "#ffffff".into(),
            font_family: None,
            bold: Some(true),
            italic: None,
            underline: None,
            background: Some("#000000".into()),
            align: None,
            opacity: Some(0.9),
        });
        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 3.0, 1.0)],
            effects: Some(vec![fx]),
        };
        let (dur, _, _) = apply_and_probe(plan, 10.0).await.unwrap();
        assert!(dur > 2.7 && dur < 3.3, "expected ~3s with text, got {dur}s");
    }

    #[tokio::test]
    async fn integration_fade_effect() {
        if !ffmpeg_ok() {
            return;
        }
        let mut fx = base_effect("fade", 1.0, 2.0);
        fx.fade = Some(FadeData {
            color: "#000000".into(),
            opacity: 0.7,
        });
        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 3.0, 1.0)],
            effects: Some(vec![fx]),
        };
        let (dur, _, _) = apply_and_probe(plan, 10.0).await.unwrap();
        assert!(dur > 2.7 && dur < 3.3, "expected ~3s with fade, got {dur}s");
    }

    #[tokio::test]
    async fn integration_multiple_effects_chained() {
        if !ffmpeg_ok() || !drawtext_available() {
            return;
        }
        let mut blur = base_effect("blur", 0.0, 3.0);
        blur.blur = Some(BlurData {
            x: 0.0,
            y: 0.0,
            width: 0.3,
            height: 0.3,
            radius: 10.0,
            invert: None,
        });
        let mut text = base_effect("text", 0.0, 3.0);
        text.text = Some(TextData {
            content: "Chained".into(),
            x: 0.5,
            y: 0.5,
            font_size: 8.0,
            color: "#ffffff".into(),
            font_family: None,
            bold: None,
            italic: None,
            underline: None,
            background: None,
            align: None,
            opacity: None,
        });
        let mut fade = base_effect("fade", 0.0, 3.0);
        fade.fade = Some(FadeData {
            color: "#ffffff".into(),
            opacity: 0.2,
        });
        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 3.0, 1.0)],
            effects: Some(vec![blur, text, fade]),
        };
        let (dur, _, _) = apply_and_probe(plan, 10.0).await.unwrap();
        assert!(
            dur > 2.7 && dur < 3.3,
            "expected ~3s with chained effects, got {dur}s"
        );
    }

    #[tokio::test]
    async fn integration_freeze_clip() {
        if !ffmpeg_ok() {
            return;
        }
        // Freeze clip at 2.0s for 2.0s duration
        let mut freeze = simple_clip(0.0, 2.0, 1.0);
        freeze.clip_type = Some("freeze".into());
        freeze.freeze_source_time = Some(2.0);
        freeze.freeze_duration = Some(2.0);

        // Concat with a normal clip so output has both
        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 2.0, 1.0), freeze],
            effects: None,
        };
        let (dur, _, _) = apply_and_probe(plan, 10.0).await.unwrap();
        // 2s normal + 2s freeze = 4s (freeze has no audio so audio may be shorter)
        assert!(dur > 3.5 && dur < 4.5, "expected ~4s, got {dur}s");
    }

    #[tokio::test]
    async fn integration_merge_replace_audio() {
        if !ffmpeg_ok() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let video = dir.path().join("v.mp4");
        let audio = dir.path().join("a.mp4");
        let output = dir.path().join("o.mp4");

        make_test_video(&video, 5.0).await.unwrap();
        make_test_video(&audio, 5.0).await.unwrap();

        let res = merge_audio_video(
            video.to_str().unwrap(),
            audio.to_str().unwrap(),
            output.to_str().unwrap(),
            true, // replace
            |_| {},
        )
        .await;
        assert!(res.is_ok(), "merge failed: {:?}", res.err());
        let meta = video_engine::probe_video(&output).await.unwrap();
        assert!(
            (meta.duration_seconds - 5.0).abs() < 0.5,
            "expected ~5s, got {}s",
            meta.duration_seconds
        );
    }

    #[tokio::test]
    async fn integration_merge_mix_audio() {
        if !ffmpeg_ok() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let video = dir.path().join("v.mp4");
        let audio = dir.path().join("a.mp4");
        let output = dir.path().join("o.mp4");

        make_test_video(&video, 5.0).await.unwrap();
        make_test_video(&audio, 5.0).await.unwrap();

        let res = merge_audio_video(
            video.to_str().unwrap(),
            audio.to_str().unwrap(),
            output.to_str().unwrap(),
            false, // mix
            |_| {},
        )
        .await;
        assert!(res.is_ok(), "merge mix failed: {:?}", res.err());
    }

    #[tokio::test]
    async fn integration_full_recomposition() {
        // End-to-end: apply_edits → merge_audio_video. Confirms the final
        // output reflects the edits and is muxed with the provided audio.
        if !ffmpeg_ok() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.mp4");
        let edited = dir.path().join("edited.mp4");
        let audio = dir.path().join("narration.mp4");
        let final_out = dir.path().join("final.mp4");

        make_test_video(&input, 10.0).await.unwrap();
        make_test_video(&audio, 4.0).await.unwrap();

        // Edit: concat two trimmed segments + one spotlight effect
        let mut fx = base_effect("spotlight", 0.5, 2.0);
        fx.spotlight = Some(SpotlightData {
            x: 0.5,
            y: 0.5,
            radius: 0.2,
            dim_opacity: 0.5,
        });
        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 2.0, 1.0), simple_clip(3.0, 5.0, 1.0)],
            effects: Some(vec![fx]),
        };

        apply_edits(
            input.to_str().unwrap(),
            edited.to_str().unwrap(),
            &plan,
            |_| {},
        )
        .await
        .unwrap();

        let edited_meta = video_engine::probe_video(&edited).await.unwrap();
        assert!(
            (edited_meta.duration_seconds - 4.0).abs() < 0.5,
            "edited should be ~4s, got {}s",
            edited_meta.duration_seconds
        );

        // Replace audio with narration track
        merge_audio_video(
            edited.to_str().unwrap(),
            audio.to_str().unwrap(),
            final_out.to_str().unwrap(),
            true,
            |_| {},
        )
        .await
        .unwrap();

        let final_meta = video_engine::probe_video(&final_out).await.unwrap();
        assert!(
            (final_meta.duration_seconds - 4.0).abs() < 0.5,
            "final should be ~4s, got {}s",
            final_meta.duration_seconds
        );
    }

    // ── Resolution + codec preservation ───────────────────────────────

    #[tokio::test]
    async fn integration_resolution_preserved_simple_trim() {
        if !ffmpeg_ok() {
            return;
        }
        let plan = VideoEditPlan {
            clips: vec![simple_clip(2.0, 6.0, 1.0)],
            effects: None,
        };
        let (_, w, h) = apply_and_probe(plan, 10.0).await.unwrap();
        assert_eq!((w, h), (320, 240));
    }

    #[tokio::test]
    async fn integration_resolution_preserved_with_speed() {
        if !ffmpeg_ok() {
            return;
        }
        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 4.0, 2.0)],
            effects: None,
        };
        let (_, w, h) = apply_and_probe(plan, 10.0).await.unwrap();
        assert_eq!((w, h), (320, 240));
    }

    #[tokio::test]
    async fn integration_resolution_preserved_with_concat() {
        if !ffmpeg_ok() {
            return;
        }
        let plan = VideoEditPlan {
            clips: vec![
                simple_clip(0.0, 2.0, 1.0),
                simple_clip(3.0, 5.0, 2.0),
                simple_clip(6.0, 8.0, 1.0),
            ],
            effects: None,
        };
        let (_, w, h) = apply_and_probe(plan, 10.0).await.unwrap();
        assert_eq!((w, h), (320, 240));
    }

    #[tokio::test]
    async fn integration_resolution_preserved_with_effects() {
        if !ffmpeg_ok() {
            return;
        }
        let mut blur = base_effect("blur", 0.0, 3.0);
        blur.blur = Some(BlurData {
            x: 0.1,
            y: 0.1,
            width: 0.3,
            height: 0.3,
            radius: 10.0,
            invert: None,
        });
        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 3.0, 1.0)],
            effects: Some(vec![blur]),
        };
        let (_, w, h) = apply_and_probe(plan, 10.0).await.unwrap();
        assert_eq!((w, h), (320, 240));
    }

    #[tokio::test]
    async fn integration_resolution_preserved_with_zoom_pan() {
        if !ffmpeg_ok() {
            return;
        }
        let mut clip = simple_clip(0.0, 4.0, 1.0);
        clip.zoom_pan = Some(ZoomPanEffect {
            start_region: crate::models::ZoomRegion {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            end_region: crate::models::ZoomRegion {
                x: 0.25,
                y: 0.25,
                width: 0.5,
                height: 0.5,
            },
            easing: EasingPreset::Linear,
        });
        let plan = VideoEditPlan {
            clips: vec![clip],
            effects: None,
        };
        let (_, w, h) = apply_and_probe(plan, 10.0).await.unwrap();
        // Zoom filter scales back to original dimensions
        assert_eq!((w, h), (320, 240));
    }

    #[tokio::test]
    async fn integration_output_codec_is_h264() {
        if !ffmpeg_ok() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.mp4");
        let output = dir.path().join("output.mp4");
        make_test_video(&input, 5.0).await.unwrap();

        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 3.0, 2.0)],
            effects: None,
        };
        apply_edits(
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            &plan,
            |_| {},
        )
        .await
        .unwrap();

        let meta = video_engine::probe_video(&output).await.unwrap();
        assert_eq!(meta.codec, "h264", "expected h264, got {}", meta.codec);
    }

    #[tokio::test]
    async fn integration_quality_file_size_reasonable() {
        // Sanity check: CRF 12 on a simple trimmed video should produce a file
        // larger than stream-copy but still sensible (not wildly inflated).
        if !ffmpeg_ok() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.mp4");
        let output = dir.path().join("output.mp4");
        make_test_video(&input, 5.0).await.unwrap();

        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 3.0, 2.0)],
            effects: None,
        };
        apply_edits(
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            &plan,
            |_| {},
        )
        .await
        .unwrap();

        let size = tokio::fs::metadata(&output).await.unwrap().len();
        // A 1.5s 320x240 h264 clip should be at least a few hundred bytes
        // (valid container with headers) and less than 10MB (not wildly inflated)
        assert!(size > 500, "output suspiciously small: {size} bytes");
        assert!(size < 10_000_000, "output suspiciously large: {size} bytes");
    }

    // ── burn_subtitles ────────────────────────────────────────────────

    async fn write_test_srt(path: &Path, content: &str) {
        tokio::fs::write(path, content).await.unwrap();
    }

    #[tokio::test]
    async fn integration_burn_subtitles_basic() {
        if !ffmpeg_ok() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.mp4");
        let srt_path = dir.path().join("sub.srt");
        let output = dir.path().join("out.mp4");
        make_test_video(&input, 5.0).await.unwrap();
        write_test_srt(
            &srt_path,
            "1\n00:00:00,000 --> 00:00:02,000\nFirst line\n\n\
             2\n00:00:02,000 --> 00:00:04,000\nSecond line\n\n",
        )
        .await;

        let style = SubtitleStyle::default();
        let res = burn_subtitles(
            input.to_str().unwrap(),
            srt_path.to_str().unwrap(),
            output.to_str().unwrap(),
            &style,
            |_| {},
        )
        .await;
        assert!(res.is_ok(), "burn_subtitles failed: {:?}", res.err());
        let meta = video_engine::probe_video(&output).await.unwrap();
        assert!(
            (meta.duration_seconds - 5.0).abs() < 0.3,
            "expected ~5s, got {}",
            meta.duration_seconds
        );
        assert_eq!((meta.width, meta.height), (320, 240));
    }

    #[tokio::test]
    async fn integration_burn_subtitles_with_custom_style() {
        if !ffmpeg_ok() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.mp4");
        let srt_path = dir.path().join("sub.srt");
        let output = dir.path().join("out.mp4");
        make_test_video(&input, 3.0).await.unwrap();
        write_test_srt(
            &srt_path,
            "1\n00:00:00,000 --> 00:00:02,000\nStyled subtitle\n\n",
        )
        .await;

        let style = SubtitleStyle {
            font_size: 28,
            color: "#ffff00".into(),
            outline_color: "#000000".into(),
            outline: 3,
            position: "top".into(),
        };
        let res = burn_subtitles(
            input.to_str().unwrap(),
            srt_path.to_str().unwrap(),
            output.to_str().unwrap(),
            &style,
            |_| {},
        )
        .await;
        assert!(
            res.is_ok(),
            "burn_subtitles with style failed: {:?}",
            res.err()
        );
    }

    #[tokio::test]
    async fn integration_burn_subtitles_with_unicode() {
        if !ffmpeg_ok() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.mp4");
        let srt_path = dir.path().join("sub.srt");
        let output = dir.path().join("out.mp4");
        make_test_video(&input, 3.0).await.unwrap();
        write_test_srt(
            &srt_path,
            "1\n00:00:00,000 --> 00:00:02,000\n日本語字幕\n\n",
        )
        .await;

        let style = SubtitleStyle::default();
        let res = burn_subtitles(
            input.to_str().unwrap(),
            srt_path.to_str().unwrap(),
            output.to_str().unwrap(),
            &style,
            |_| {},
        )
        .await;
        assert!(res.is_ok(), "unicode SRT should work: {:?}", res.err());
    }

    // ── extract_edit_thumbnails ──────────────────────────────────────

    #[tokio::test]
    async fn integration_extract_edit_thumbnails_count() {
        if !ffmpeg_ok() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.mp4");
        let out_dir = dir.path().join("thumbs");
        std::fs::create_dir_all(&out_dir).unwrap();
        make_test_video(&input, 10.0).await.unwrap();

        let thumbs = extract_edit_thumbnails(input.to_str().unwrap(), out_dir.to_str().unwrap(), 5)
            .await
            .unwrap();
        assert_eq!(
            thumbs.len(),
            5,
            "expected 5 thumbnails, got {}",
            thumbs.len()
        );
        // Each thumbnail should exist and be non-empty
        for t in &thumbs {
            let size = tokio::fs::metadata(t).await.unwrap().len();
            assert!(size > 100, "thumbnail {t} too small: {size} bytes");
        }
    }

    #[tokio::test]
    async fn integration_extract_edit_thumbnails_cache_hit() {
        if !ffmpeg_ok() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.mp4");
        let out_dir = dir.path().join("thumbs_cache");
        std::fs::create_dir_all(&out_dir).unwrap();
        make_test_video(&input, 5.0).await.unwrap();

        // First call populates the cache
        let first_start = std::time::Instant::now();
        let thumbs1 =
            extract_edit_thumbnails(input.to_str().unwrap(), out_dir.to_str().unwrap(), 5)
                .await
                .unwrap();
        let first_elapsed = first_start.elapsed();
        assert_eq!(thumbs1.len(), 5);

        // Second call with same inputs should hit cache and be substantially faster
        let second_start = std::time::Instant::now();
        let thumbs2 =
            extract_edit_thumbnails(input.to_str().unwrap(), out_dir.to_str().unwrap(), 5)
                .await
                .unwrap();
        let second_elapsed = second_start.elapsed();
        assert_eq!(thumbs2.len(), 5);
        // Cache hit should be at least 5x faster than the ffmpeg run
        assert!(
            second_elapsed * 5 < first_elapsed,
            "expected cache hit (2nd={:?}) to be much faster than cold (1st={:?})",
            second_elapsed,
            first_elapsed
        );
    }

    #[tokio::test]
    async fn integration_extract_single_frame() {
        if !ffmpeg_ok() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.mp4");
        let out = dir.path().join("frame.jpg");
        make_test_video(&input, 10.0).await.unwrap();

        let result =
            extract_single_frame(input.to_str().unwrap(), 5.0, out.to_str().unwrap()).await;
        assert!(
            result.is_ok(),
            "extract_single_frame failed: {:?}",
            result.err()
        );
        let size = tokio::fs::metadata(&out).await.unwrap().len();
        assert!(size > 100, "frame file too small: {size} bytes");
    }

    // ── End-to-end lossless pipeline ─────────────────────────────────

    /// Walk the full export pipeline: apply_edits → merge_audio_video →
    /// burn_subtitles. Confirm every edit propagates through and the final
    /// output's resolution matches the source.
    #[tokio::test]
    async fn integration_e2e_edits_propagate_to_export() {
        if !ffmpeg_ok() || !drawtext_available() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.mp4");
        let edited = dir.path().join("edited.mp4");
        let audio = dir.path().join("narration.mp4");
        let merged = dir.path().join("merged.mp4");
        let srt = dir.path().join("subs.srt");
        let final_out = dir.path().join("final.mp4");

        make_test_video(&input, 10.0).await.unwrap();
        make_test_video(&audio, 6.0).await.unwrap();
        tokio::fs::write(
            &srt,
            "1\n00:00:00,000 --> 00:00:02,000\nOpening line\n\n\
             2\n00:00:03,000 --> 00:00:05,000\n日本語字幕\n\n",
        )
        .await
        .unwrap();

        // Step 1: edit with trim + speed + zoom-pan + spotlight + text
        let mut clip1 = simple_clip(0.0, 3.0, 1.0);
        clip1.zoom_pan = Some(ZoomPanEffect {
            start_region: crate::models::ZoomRegion {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            end_region: crate::models::ZoomRegion {
                x: 0.3,
                y: 0.3,
                width: 0.4,
                height: 0.4,
            },
            easing: EasingPreset::EaseInOut,
        });
        let clip2 = simple_clip(4.0, 7.0, 2.0); // 3s source → 1.5s output
                                                // Output timeline: clip1 = 3s, clip2 = 1.5s → total 4.5s
        let mut spotlight = base_effect("spotlight", 0.5, 2.0);
        spotlight.spotlight = Some(SpotlightData {
            x: 0.5,
            y: 0.5,
            radius: 0.2,
            dim_opacity: 0.6,
        });
        let mut text = base_effect("text", 3.0, 4.5);
        text.text = Some(TextData {
            content: "End of clip".into(),
            x: 0.5,
            y: 0.9,
            font_size: 5.0,
            color: "#ffffff".into(),
            font_family: None,
            bold: Some(true),
            italic: None,
            underline: None,
            background: None,
            align: None,
            opacity: Some(1.0),
        });

        let plan = VideoEditPlan {
            clips: vec![clip1, clip2],
            effects: Some(vec![spotlight, text]),
        };
        apply_edits(
            input.to_str().unwrap(),
            edited.to_str().unwrap(),
            &plan,
            |_| {},
        )
        .await
        .unwrap();

        let edited_meta = video_engine::probe_video(&edited).await.unwrap();
        // Expected: 3s + 1.5s = 4.5s (±0.5 for encode jitter)
        assert!(
            (edited_meta.duration_seconds - 4.5).abs() < 0.6,
            "edited duration drift: expected ~4.5s, got {}",
            edited_meta.duration_seconds
        );
        assert_eq!(
            (edited_meta.width, edited_meta.height),
            (320, 240),
            "resolution must be preserved through apply_edits"
        );

        // Step 2: merge with narration audio (replace original)
        merge_audio_video(
            edited.to_str().unwrap(),
            audio.to_str().unwrap(),
            merged.to_str().unwrap(),
            true,
            |_| {},
        )
        .await
        .unwrap();

        let merged_meta = video_engine::probe_video(&merged).await.unwrap();
        // Merge preserves video via -c:v copy → duration & resolution unchanged
        assert!(
            (merged_meta.duration_seconds - edited_meta.duration_seconds).abs() < 0.3,
            "merge changed duration: was {}, now {}",
            edited_meta.duration_seconds,
            merged_meta.duration_seconds
        );
        assert_eq!(
            (merged_meta.width, merged_meta.height),
            (edited_meta.width, edited_meta.height),
            "merge must preserve resolution"
        );

        // Step 3: burn subtitles
        let style = SubtitleStyle::default();
        burn_subtitles(
            merged.to_str().unwrap(),
            srt.to_str().unwrap(),
            final_out.to_str().unwrap(),
            &style,
            |_| {},
        )
        .await
        .unwrap();

        let final_meta = video_engine::probe_video(&final_out).await.unwrap();
        assert!(
            (final_meta.duration_seconds - merged_meta.duration_seconds).abs() < 0.3,
            "burn_subtitles changed duration: was {}, now {}",
            merged_meta.duration_seconds,
            final_meta.duration_seconds
        );
        assert_eq!(
            (final_meta.width, final_meta.height),
            (merged_meta.width, merged_meta.height),
            "burn_subtitles must preserve resolution"
        );
        assert_eq!(final_meta.codec, "h264", "final output must be h264");
    }

    /// Regression guard: a single clip with speed=1 and an overlay effect
    /// (blur/spotlight/fade) must still run the effects pass — the fast-path
    /// code previously skipped effects, silently producing an unedited output.
    #[tokio::test]
    async fn integration_single_clip_with_overlay_effect_not_dropped() {
        if !ffmpeg_ok() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.mp4");
        let output = dir.path().join("out.mp4");
        make_test_video(&input, 5.0).await.unwrap();

        // Source size before any edit
        let src_size = tokio::fs::metadata(&input).await.unwrap().len();

        // Single clip, speed=1, no zoom/freeze, but WITH a fade effect.
        // Before the fix, this went through the fast path and effects were
        // silently dropped.
        let mut fade = base_effect("fade", 0.0, 5.0);
        fade.fade = Some(FadeData {
            color: "#000000".into(),
            opacity: 0.5,
        });
        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 5.0, 1.0)],
            effects: Some(vec![fade]),
        };
        apply_edits(
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            &plan,
            |_| {},
        )
        .await
        .unwrap();

        // If the fast path ran, it would be `-c copy` so size ≈ src.
        // With CRF 0 + fade drawbox overlay, size should be different
        // (either larger due to the CRF 0 re-encode, or same if encoder is
        // very efficient). The key check: the output actually went through
        // the effects pass. We can't easily detect the overlay visually in
        // a unit test, but we can probe that the output exists and isn't a
        // byte-for-byte copy of the input.
        let out_size = tokio::fs::metadata(&output).await.unwrap().len();
        assert!(out_size > 0);
        // The CRF-0 lossless re-encode will be significantly larger than
        // a CRF-36 testsrc input; if sizes are nearly identical something
        // copied the source without processing.
        let ratio = out_size as f64 / src_size as f64;
        assert!(
            ratio > 2.0,
            "output/input size ratio = {ratio:.2} — suggests fast path ran and skipped effects"
        );
    }

    /// Lossless verification: when a clip has `speed=1.0`, no zoom, no fps
    /// override, no effects, the output's video bitrate should be at least
    /// as high as the source (CRF 0 = bit-exact). This is a weak check (same
    /// decode but ffprobe counts both bitrates, good enough as a regression
    /// guard against accidentally re-introducing CRF > 0).
    #[tokio::test]
    async fn integration_lossless_encode_simple_trim() {
        if !ffmpeg_ok() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.mp4");
        let output = dir.path().join("out.mp4");
        make_test_video(&input, 5.0).await.unwrap();

        // Full-source single clip with speed=1 falls through the fast path
        // (stream-copy). But for multi-clip we re-encode at CRF 0 — so use
        // two clips to force the re-encode path.
        let plan = VideoEditPlan {
            clips: vec![simple_clip(0.0, 2.5, 1.0), simple_clip(2.5, 5.0, 1.0)],
            effects: None,
        };
        apply_edits(
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            &plan,
            |_| {},
        )
        .await
        .unwrap();

        let src_size = tokio::fs::metadata(&input).await.unwrap().len();
        let out_size = tokio::fs::metadata(&output).await.unwrap().len();
        // Lossless output from testsrc should be AT LEAST as large as the
        // heavily-compressed source. A CRF-0 encode cannot produce a smaller
        // file than the lossy input unless something is stripped.
        assert!(
            out_size >= src_size,
            "lossless output ({out_size}) unexpectedly smaller than source ({src_size}) — \
             likely indicates we're not actually encoding lossless"
        );
    }
}
