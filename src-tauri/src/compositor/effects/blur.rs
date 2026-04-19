//! Rectangular blur (or inverted blur, where the rect stays sharp and the
//! surrounding area blurs).
//!
//! Replaces `video_edit.rs::build_blur_filter` (lines 398-436), which used
//! ffmpeg `boxblur` + `split` + `overlay`. We do the box blur in pure Rust
//! via `image::imageops::blur`, then composite back onto the canvas.

use image::{DynamicImage, ImageBuffer, RgbaImage};
use tiny_skia::{IntRect, Pixmap, PixmapPaint, Transform};

/// Apply a blur to a sub-rectangle of the canvas (or to everything outside
/// it when `invert=true`).
///
/// All position params are normalized [0, 1] of the canvas dimensions.
/// `radius_n` is the blur radius — accepted in either pixels (when > 1.0)
/// or normalized to canvas width.
#[allow(clippy::too_many_arguments)]
pub fn apply_blur(
    canvas: &mut Pixmap,
    x_n: f32,
    y_n: f32,
    w_n: f32,
    h_n: f32,
    radius_n: f32,
    invert: bool,
    effect_alpha: f32,
) {
    if effect_alpha <= 0.001 {
        return;
    }
    let cw = canvas.width() as f32;
    let ch = canvas.height() as f32;
    if cw <= 0.0 || ch <= 0.0 {
        return;
    }

    let x = (x_n.clamp(0.0, 1.0) * cw) as i32;
    let y = (y_n.clamp(0.0, 1.0) * ch) as i32;
    let w = (w_n.clamp(0.0, 1.0) * cw).round() as u32;
    let h = (h_n.clamp(0.0, 1.0) * ch).round() as u32;
    if w == 0 || h == 0 {
        return;
    }
    // pixel radius
    let radius = if radius_n > 1.0 {
        radius_n
    } else {
        radius_n * cw
    }
    .clamp(0.5, 200.0);

    if invert {
        // Blur the whole canvas, then paste the original sharp rect back on top.
        let original = match clone_pixmap(canvas) {
            Some(p) => p,
            None => return,
        };
        let blurred = blur_pixmap(canvas, radius);
        canvas.fill(tiny_skia::Color::TRANSPARENT);
        let paint = PixmapPaint::default();
        canvas.draw_pixmap(0, 0, blurred.as_ref(), &paint, Transform::identity(), None);
        // Paste the un-blurred rect back on top.
        if let Some(rect) = IntRect::from_xywh(x, y, w, h) {
            if let Some(sub) = sub_pixmap(&original, &rect) {
                canvas.draw_pixmap(x, y, sub.as_ref(), &paint, Transform::identity(), None);
            }
        }
    } else {
        // Blur just the rect.
        let rect = match IntRect::from_xywh(x, y, w, h) {
            Some(r) => r,
            None => return,
        };
        let sub = match sub_pixmap(canvas, &rect) {
            Some(p) => p,
            None => return,
        };
        let blurred = blur_pixmap(&sub, radius);
        let paint = PixmapPaint::default();
        canvas.draw_pixmap(x, y, blurred.as_ref(), &paint, Transform::identity(), None);
    }
}

fn clone_pixmap(src: &Pixmap) -> Option<Pixmap> {
    let mut dst = Pixmap::new(src.width(), src.height())?;
    dst.data_mut().copy_from_slice(src.data());
    Some(dst)
}

fn sub_pixmap(src: &Pixmap, rect: &IntRect) -> Option<Pixmap> {
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

fn blur_pixmap(src: &Pixmap, radius: f32) -> Pixmap {
    let w = src.width();
    let h = src.height();
    let img: RgbaImage = ImageBuffer::from_raw(w, h, src.data().to_vec()).expect("rgba buf size");
    let blurred = image::imageops::blur(&DynamicImage::ImageRgba8(img).into_rgba8(), radius);
    let mut dst = Pixmap::new(w, h).expect("blur pixmap alloc");
    dst.data_mut().copy_from_slice(&blurred);
    dst
}
