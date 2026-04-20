//! Zoom-pan (Ken Burns) — animated affine warp.
//!
//! Replaces `video_edit.rs::build_zoompan_filter_for_video` (lines 1021-1061)
//! and the constant-size-crop "Reinitializing filters" workaround (lines
//! 612-617). Here we just compute the per-frame source rectangle in plain
//! Rust and ask tiny-skia to draw the source pixmap with that crop scaled
//! to the canvas.
//!
//! Coordinate convention: regions are normalized 0..1 of the source frame.

use tiny_skia::{FilterQuality, IntRect, Pixmap, Rect, Transform};

use crate::compositor::keyframe::{ease, Interp};
use crate::models::ZoomPanEffect;

/// Render `source` onto `canvas` with a zoom-pan transform that linearly
/// (or eased) interpolates between `effect.start_region` and `effect.end_region`.
///
/// `progress` is normalized 0..1 representing how far through the effect
/// window we are (caller applies transitions / reverse / etc).
pub fn apply_zoom_pan(canvas: &mut Pixmap, source: &Pixmap, effect: &ZoomPanEffect, progress: f32) {
    let interp: Interp = effect.easing.into();
    let p = ease(progress.clamp(0.0, 1.0), interp);

    let s = &effect.start_region;
    let e = &effect.end_region;

    // Lerp normalized region.
    let rx = (s.x + (e.x - s.x) * p as f64).clamp(0.0, 1.0) as f32;
    let ry = (s.y + (e.y - s.y) * p as f64).clamp(0.0, 1.0) as f32;
    let rw = (s.width + (e.width - s.width) * p as f64).clamp(0.05, 1.0) as f32;
    let rh = (s.height + (e.height - s.height) * p as f64).clamp(0.05, 1.0) as f32;

    let sw = source.width() as f32;
    let sh = source.height() as f32;
    let cw = canvas.width() as f32;
    let ch = canvas.height() as f32;

    // Crop rectangle in source pixel space.
    let crop_x = (rx * sw).max(0.0);
    let crop_y = (ry * sh).max(0.0);
    let crop_w = (rw * sw).clamp(2.0, sw - crop_x);
    let crop_h = (rh * sh).clamp(2.0, sh - crop_y);

    // Build a sub-pixmap (lanczos-quality scale via image crate is overkill;
    // tiny-skia's bilinear is good enough at typical export resolutions and
    // matches the existing zoompan filter's perceived quality).
    let crop_rect = IntRect::from_xywh(
        crop_x.round() as i32,
        crop_y.round() as i32,
        crop_w.round() as u32,
        crop_h.round() as u32,
    );
    let sub = match crop_rect.and_then(|r| extract_sub(source, &r)) {
        Some(p) => p,
        None => return,
    };

    // Scale sub → canvas.
    let scale_x = cw / sub.width() as f32;
    let scale_y = ch / sub.height() as f32;
    let transform = Transform::from_scale(scale_x, scale_y);

    canvas.fill(tiny_skia::Color::BLACK);
    // draw_pixmap uses integer translation for the destination origin (0,0)
    // and applies `transform` to the *source* pixmap before sampling.
    let pattern = tiny_skia::Pattern::new(
        sub.as_ref(),
        tiny_skia::SpreadMode::Pad,
        FilterQuality::Bilinear,
        1.0,
        transform,
    );
    let fill_paint = tiny_skia::Paint {
        shader: pattern,
        anti_alias: false,
        ..tiny_skia::Paint::default()
    };
    let dst_rect = Rect::from_xywh(0.0, 0.0, cw, ch).expect("dst rect");
    canvas.fill_rect(dst_rect, &fill_paint, Transform::identity(), None);
}

fn extract_sub(src: &Pixmap, rect: &IntRect) -> Option<Pixmap> {
    let sw = src.width() as i32;
    let sh = src.height() as i32;
    let x0 = rect.x().max(0);
    let y0 = rect.y().max(0);
    let x1 = (rect.x() + rect.width() as i32).min(sw);
    let y1 = (rect.y() + rect.height() as i32).min(sh);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let w = (x1 - x0) as u32;
    let h = (y1 - y0) as u32;
    let mut dst = Pixmap::new(w, h)?;
    let src_data = src.data();
    let dst_data = dst.data_mut();
    let src_w = src.width() as usize;
    for row in 0..h {
        let sy = y0 as usize + row as usize;
        let so = (sy * src_w + x0 as usize) * 4;
        let dst_off = (row as usize) * (w as usize) * 4;
        let row_bytes = (w as usize) * 4;
        dst_data[dst_off..dst_off + row_bytes].copy_from_slice(&src_data[so..so + row_bytes]);
    }
    Some(dst)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{EasingPreset, ZoomRegion};

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> Pixmap {
        let mut p = Pixmap::new(w, h).unwrap();
        let d = p.data_mut();
        for chunk in d.chunks_exact_mut(4) {
            chunk.copy_from_slice(&rgba);
        }
        p
    }

    #[test]
    fn full_region_renders_full_source() {
        let src = solid(20, 20, [10, 20, 30, 255]);
        let mut canvas = Pixmap::new(20, 20).unwrap();
        let effect = ZoomPanEffect {
            start_region: ZoomRegion {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            end_region: ZoomRegion {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            easing: EasingPreset::Linear,
        };
        apply_zoom_pan(&mut canvas, &src, &effect, 0.0);
        // Some pixel in the middle should have the source colour.
        let off = (10 * 20 + 10) * 4;
        let px = &canvas.data()[off..off + 4];
        assert_eq!(px[0], 10);
        assert_eq!(px[1], 20);
        assert_eq!(px[2], 30);
    }

    #[test]
    fn progress_clamping_does_not_panic() {
        let src = solid(20, 20, [100, 100, 100, 255]);
        let mut canvas = Pixmap::new(20, 20).unwrap();
        let effect = ZoomPanEffect {
            start_region: ZoomRegion {
                x: 0.0,
                y: 0.0,
                width: 0.5,
                height: 0.5,
            },
            end_region: ZoomRegion {
                x: 0.5,
                y: 0.5,
                width: 0.5,
                height: 0.5,
            },
            easing: EasingPreset::EaseInOut,
        };
        apply_zoom_pan(&mut canvas, &src, &effect, -1.0);
        apply_zoom_pan(&mut canvas, &src, &effect, 0.0);
        apply_zoom_pan(&mut canvas, &src, &effect, 0.5);
        apply_zoom_pan(&mut canvas, &src, &effect, 1.0);
        apply_zoom_pan(&mut canvas, &src, &effect, 2.0);
    }
}
