//! Full-frame solid-colour fade overlay.
//!
//! Replaces ffmpeg `drawbox` (`video_edit.rs::build_effects_filter` lines
//! 554–565). Time animation is handled by the caller — this function takes a
//! fully-resolved RGBA colour + opacity and blends it across the whole canvas.

use tiny_skia::{Color, Paint, Pixmap, Rect, Transform};

use super::parse_hex_rgba;

/// Blend a translucent rectangle over the whole pixmap.
/// `opacity` in [0, 1]; `hex` accepts `#RRGGBB` or `#RRGGBBAA`.
pub fn apply_fade(canvas: &mut Pixmap, hex: &str, opacity: f32) {
    let opacity = opacity.clamp(0.0, 1.0);
    if opacity <= 0.001 {
        return;
    }
    let (r, g, b, a) = parse_hex_rgba(hex);
    // Compose user-supplied colour-alpha with the effect-level opacity.
    let final_a = (a as f32 / 255.0) * opacity;
    if final_a <= 0.001 {
        return;
    }
    let mut paint = Paint::default();
    // tiny-skia takes premultiplied alpha for fills via Color::from_rgba
    paint.set_color(
        Color::from_rgba(
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            final_a,
        )
        .unwrap_or(Color::TRANSPARENT),
    );
    let rect = Rect::from_xywh(0.0, 0.0, canvas.width() as f32, canvas.height() as f32)
        .expect("canvas rect");
    canvas.fill_rect(rect, &paint, Transform::identity(), None);
}
