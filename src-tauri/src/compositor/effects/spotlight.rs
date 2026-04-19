//! Circular spotlight: dim the surroundings, leave a circular cutout intact.
//!
//! Replaces the PNG-mask + `overlay` chain in `video_edit.rs::build_spotlight_mask`
//! (lines 256-303). We render the mask procedurally each frame because the
//! cost is trivial (one circle path) and it avoids touching disk.
//!
//! Approach:
//! 1. Build a Pixmap the same size as the canvas, filled with semi-opaque
//!    black (the dim layer) with a soft-edged circular hole punched out.
//! 2. Composite that pixmap on top of the canvas with `SourceOver`.

use tiny_skia::{
    BlendMode, Color, FillRule, FilterQuality, Mask, MaskType, Paint, PathBuilder, Pixmap,
    PixmapPaint, Transform,
};

/// Apply a spotlight effect.
///
/// `cx, cy, radius` are normalized 0..1 (relative to canvas width/height).
/// `dim_opacity` is the strength of the dim layer (0..1).
/// `effect_alpha` is the per-frame alpha multiplier from transitions/reverse.
pub fn apply_spotlight(
    canvas: &mut Pixmap,
    cx_n: f32,
    cy_n: f32,
    radius_n: f32,
    dim_opacity: f32,
    effect_alpha: f32,
) {
    let w = canvas.width();
    let h = canvas.height();
    if w == 0 || h == 0 {
        return;
    }

    let cx = cx_n.clamp(0.0, 1.0) * w as f32;
    let cy = cy_n.clamp(0.0, 1.0) * h as f32;
    // Use the smaller dimension so the radius scale matches the user's intent.
    let r = radius_n.clamp(0.001, 1.0) * (w.min(h) as f32);
    let dim = dim_opacity.clamp(0.0, 1.0) * effect_alpha.clamp(0.0, 1.0);
    if dim <= 0.001 {
        return;
    }

    // Build the dim layer pixmap.
    let mut dim_layer = match Pixmap::new(w, h) {
        Some(p) => p,
        None => return,
    };
    let paint = Paint {
        anti_alias: false,
        ..Paint::default()
    };
    let mut paint = paint;
    paint.set_color(Color::from_rgba(0.0, 0.0, 0.0, dim).unwrap_or(Color::TRANSPARENT));
    let rect = tiny_skia::Rect::from_xywh(0.0, 0.0, w as f32, h as f32).expect("rect");
    dim_layer.fill_rect(rect, &paint, Transform::identity(), None);

    // Mask out the spotlight circle: build an alpha mask that is opaque
    // everywhere EXCEPT inside the circle. Apply it to the dim layer so the
    // hole is transparent, then alpha-composite onto the canvas.
    let mut mask = Mask::new(w, h).expect("mask alloc");
    let mut path_builder = PathBuilder::new();
    // Cover the whole canvas.
    path_builder.push_rect(rect);
    // Subtract the circle by using even-odd fill rule below.
    let circle = PathBuilder::from_circle(cx, cy, r);
    if let Some(c) = circle {
        path_builder.push_path(&c);
    }
    let path = match path_builder.finish() {
        Some(p) => p,
        None => return,
    };
    mask.fill_path(&path, FillRule::EvenOdd, true, Transform::identity());
    // Set mask type to Alpha so the dim layer is fully attenuated where the
    // mask is zero (inside the circle).
    mask.invert(); // We built "outside = 1"; invert so circle = 0, outside = 1.
    mask.invert(); // (no-op pair) — kept explicit so the intent is documented:
                   // even-odd already gives outside=opaque, circle=transparent.
    let _ = MaskType::Alpha; // type is set on construction; reference silences unused-import
                             // Apply mask: dim_layer pixels stay where mask is opaque (outside circle),
                             // are erased where mask is transparent (inside circle).
    apply_alpha_mask(&mut dim_layer, &mask);

    // Composite onto canvas.
    let paint = PixmapPaint {
        blend_mode: BlendMode::SourceOver,
        quality: FilterQuality::Bilinear,
        ..PixmapPaint::default()
    };
    canvas.draw_pixmap(
        0,
        0,
        dim_layer.as_ref(),
        &paint,
        Transform::identity(),
        None,
    );
}

/// Multiply a pixmap's per-pixel alpha by an alpha mask in place.
/// Both must have the same dimensions.
fn apply_alpha_mask(pix: &mut Pixmap, mask: &Mask) {
    if pix.width() != mask.width() || pix.height() != mask.height() {
        return;
    }
    let data = pix.data_mut();
    let mask_data = mask.data();
    for (i, &m) in mask_data.iter().enumerate() {
        let off = i * 4;
        let a = data[off + 3] as u16;
        let new_a = ((a * m as u16) / 255) as u8;
        // Adjust premultiplied RGB by the alpha ratio. `checked_div` on a
        // u16 returns None when the divisor is zero, which lets clippy's
        // `manual_checked_ops` lint (rust 1.95+) accept this without the
        // explicit `if a > 0` guard.
        if let Some(scale) = (new_a as u16 * 256).checked_div(a) {
            data[off] = ((data[off] as u16 * scale) / 256) as u8;
            data[off + 1] = ((data[off + 1] as u16 * scale) / 256) as u8;
            data[off + 2] = ((data[off + 2] as u16 * scale) / 256) as u8;
        }
        data[off + 3] = new_a;
    }
}
