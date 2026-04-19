//! Per-effect compositors. Each module exposes a thin pure(-ish) function
//! that takes the working RGBA `Pixmap`, the effect's parameters, and any
//! pre-computed progress / alpha values, and writes its output in-place.
//!
//! Effects do NOT decide their own time animation — the orchestrator
//! (`compositor::run_effects_pass`) calls `keyframe::window_progress` first
//! and feeds the resulting `progress` / `effect_alpha` to each call.

pub mod blur;
pub mod fade;
pub mod spotlight;
pub mod text;
pub mod zoom_pan;

/// Parse `#RRGGBB` or `#RRGGBBAA` into an `(r, g, b, a)` byte tuple.
/// Falls back to opaque black on malformed input.
pub fn parse_hex_rgba(s: &str) -> (u8, u8, u8, u8) {
    let h = s.trim().trim_start_matches('#');
    let parse = |a: usize, b: usize| u8::from_str_radix(&h[a..b], 16).ok();
    if h.len() == 8 {
        if let (Some(r), Some(g), Some(b), Some(a)) =
            (parse(0, 2), parse(2, 4), parse(4, 6), parse(6, 8))
        {
            return (r, g, b, a);
        }
    } else if h.len() == 6 {
        if let (Some(r), Some(g), Some(b)) = (parse(0, 2), parse(2, 4), parse(4, 6)) {
            return (r, g, b, 255);
        }
    }
    (0, 0, 0, 255)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_short_hex() {
        assert_eq!(parse_hex_rgba("#ff0000"), (255, 0, 0, 255));
        assert_eq!(parse_hex_rgba("00ff00"), (0, 255, 0, 255));
    }

    #[test]
    fn parse_with_alpha() {
        assert_eq!(parse_hex_rgba("#0000ff80"), (0, 0, 255, 128));
    }

    #[test]
    fn malformed_falls_back() {
        assert_eq!(parse_hex_rgba("nope"), (0, 0, 0, 255));
        assert_eq!(parse_hex_rgba(""), (0, 0, 0, 255));
    }
}
