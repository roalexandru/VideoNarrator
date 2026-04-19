//! Text overlay.
//!
//! Strategy: pre-render the text content to a transparent RGBA Pixmap **once**
//! per render (via ffmpeg `drawtext` against a transparent input), cache it,
//! and alpha-composite that pixmap per frame with the current effect alpha.
//!
//! Why this approach: text content / position / font are not time-varying —
//! only the effect-level alpha (transitions / reverse) is. Pre-rendering
//! avoids embedding a font in the binary (a non-trivial ~600KB artifact +
//! licensing decision) while keeping rendered output bit-identical to what
//! users see today.
//!
//! When/if we want true per-frame text animation (typewriter, shimmer), this
//! is where to swap in `ab_glyph` / `cosmic-text` glyph rasterization on a
//! per-frame basis — the public `apply_text` signature stays the same.

use std::path::PathBuf;
use std::sync::Arc;

use tiny_skia::{Pixmap, PixmapPaint, Transform};
use tokio::process::Command;

use crate::error::NarratorError;
use crate::process_utils::CommandNoWindow;
use crate::video_edit::TextData;
use crate::video_engine;

use super::parse_hex_rgba;

pub struct PreRenderedText {
    pub pixmap: Pixmap,
}

/// Cached pre-renders keyed by a hash of (canvas_w, canvas_h, TextData JSON).
///
/// Multiple effects sharing identical params hit the cache — common when the
/// user clones a text track. The cache lives only for one export.
#[derive(Default)]
pub struct TextRenderCache {
    pub entries: std::collections::HashMap<u64, Arc<PreRenderedText>>,
}

impl TextRenderCache {
    pub async fn get_or_render(
        &mut self,
        text: &TextData,
        canvas_w: u32,
        canvas_h: u32,
    ) -> Result<Option<Arc<PreRenderedText>>, NarratorError> {
        let key = compute_text_key(text, canvas_w, canvas_h);
        if let Some(p) = self.entries.get(&key) {
            return Ok(Some(p.clone()));
        }
        // Some ffmpeg builds (notably Homebrew's default on macOS) ship
        // without libfreetype — `drawtext` isn't registered. Rather than
        // failing the entire render, log a warning and skip the text
        // overlay; every other effect still composites normally. A future
        // pure-Rust rasterizer (ab_glyph is already a dep) can replace this
        // path without changing the public surface.
        if !drawtext_available().await {
            tracing::warn!(
                "ffmpeg drawtext filter not available — skipping text overlay '{}' \
                 (rebuild ffmpeg with libfreetype to enable)",
                text.content.chars().take(40).collect::<String>()
            );
            return Ok(None);
        }
        let rendered = render_text_to_pixmap(text, canvas_w, canvas_h).await?;
        let arc = Arc::new(rendered);
        self.entries.insert(key, arc.clone());
        Ok(Some(arc))
    }

    /// Cache lookup by the same key the orchestrator will compute.
    /// Returns `None` if the entry was never inserted.
    pub fn lookup(
        &self,
        text: &TextData,
        canvas_w: u32,
        canvas_h: u32,
    ) -> Option<Arc<PreRenderedText>> {
        let key = compute_text_key(text, canvas_w, canvas_h);
        self.entries.get(&key).cloned()
    }
}

/// Cached probe: does the on-system ffmpeg expose the `drawtext` filter?
/// Result is computed once per process and cached behind an OnceCell.
async fn drawtext_available() -> bool {
    use tokio::sync::OnceCell;
    static AVAILABLE: OnceCell<bool> = OnceCell::const_new();
    *AVAILABLE
        .get_or_init(|| async {
            let Ok(ffmpeg) = video_engine::detect_ffmpeg() else {
                return false;
            };
            let Ok(output) = Command::new(ffmpeg)
                .no_window()
                .arg("-filters")
                .output()
                .await
            else {
                return false;
            };
            String::from_utf8_lossy(&output.stdout).contains("drawtext")
        })
        .await
}

fn compute_text_key(text: &TextData, w: u32, h: u32) -> u64 {
    let mut hasher = blake3::Hasher::new();
    let payload = serde_json::to_vec(text).unwrap_or_default();
    hasher.update(&payload);
    hasher.update(&w.to_le_bytes());
    hasher.update(&h.to_le_bytes());
    let bytes = hasher.finalize();
    u64::from_le_bytes(bytes.as_bytes()[..8].try_into().unwrap())
}

/// Composite a pre-rendered text pixmap onto the canvas at full coverage.
/// `effect_alpha` is the per-frame alpha multiplier from transitions/reverse.
pub fn apply_text(canvas: &mut Pixmap, text_pix: &Pixmap, effect_alpha: f32) {
    let paint = PixmapPaint {
        opacity: effect_alpha.clamp(0.0, 1.0),
        blend_mode: tiny_skia::BlendMode::SourceOver,
        ..PixmapPaint::default()
    };
    canvas.draw_pixmap(0, 0, text_pix.as_ref(), &paint, Transform::identity(), None);
}

/// Render the text overlay to a transparent canvas-sized pixmap by spawning
/// `ffmpeg -f lavfi -i color=...:size=WxH -vf drawtext=...` once.
async fn render_text_to_pixmap(
    text: &TextData,
    canvas_w: u32,
    canvas_h: u32,
) -> Result<PreRenderedText, NarratorError> {
    let ffmpeg = video_engine::detect_ffmpeg()?;
    let tmp_dir = std::env::temp_dir();
    let png_path: PathBuf = tmp_dir.join(format!("_text_overlay_{}.png", uuid::Uuid::new_v4()));

    let drawtext = build_drawtext_filter(text, canvas_w, canvas_h);

    // lavfi color filter with alpha=0 gives a transparent canvas.
    let color_input = format!("color=c=black@0.0:size={canvas_w}x{canvas_h}:duration=1:rate=1");

    let output = Command::new(ffmpeg.as_os_str())
        .no_window()
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            &color_input,
            "-vf",
            &drawtext,
            "-frames:v",
            "1",
        ])
        .arg(&png_path)
        .output()
        .await
        .map_err(|e| NarratorError::FfmpegFailed(format!("text render: {e}")))?;

    if !output.status.success() {
        let _ = tokio::fs::remove_file(&png_path).await;
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(NarratorError::FfmpegFailed(format!(
            "text drawtext failed: {stderr}"
        )));
    }

    let bytes = tokio::fs::read(&png_path).await?;
    let _ = tokio::fs::remove_file(&png_path).await;

    let img = image::load_from_memory(&bytes)
        .map_err(|e| NarratorError::FfmpegFailed(format!("text png decode: {e}")))?
        .to_rgba8();
    let (w, h) = (img.width(), img.height());
    let mut pix = Pixmap::new(w, h)
        .ok_or_else(|| NarratorError::FfmpegFailed(format!("text pixmap alloc {w}x{h}")))?;
    pix.data_mut().copy_from_slice(&img);

    // If ffmpeg gave us a smaller pixmap (rare but possible), re-frame onto
    // canvas-sized to keep the per-frame composite simple.
    if w != canvas_w || h != canvas_h {
        let mut framed = Pixmap::new(canvas_w, canvas_h)
            .ok_or_else(|| NarratorError::FfmpegFailed("text frame alloc".into()))?;
        let paint = PixmapPaint::default();
        framed.draw_pixmap(0, 0, pix.as_ref(), &paint, Transform::identity(), None);
        pix = framed;
    }

    Ok(PreRenderedText { pixmap: pix })
}

fn build_drawtext_filter(text: &TextData, canvas_w: u32, canvas_h: u32) -> String {
    let escaped = text
        .content
        .replace('\\', "\\\\")
        .replace('\'', "\\\\'")
        .replace(':', "\\:")
        .replace('%', "%%")
        .replace(['\n', '\r'], " ");

    let font_px = (text.font_size / 100.0 * canvas_h as f64).max(8.0).round() as u32;
    let (r, g, b, a) = parse_hex_rgba(&text.color);
    let opacity = text.opacity.unwrap_or(1.0).clamp(0.0, 1.0) * (a as f64 / 255.0);
    let color = format!("0x{:02X}{:02X}{:02X}@{:.3}", r, g, b, opacity);

    // Position: TextData.x/y are normalized 0..1 of canvas.
    let x_px = (text.x.clamp(0.0, 1.0) * canvas_w as f64).round() as i32;
    let y_px = (text.y.clamp(0.0, 1.0) * canvas_h as f64).round() as i32;

    let bold = text.bold.unwrap_or(false);
    let borderw = if bold { 2 } else { 1 };

    let mut parts = vec![
        format!("text='{escaped}'"),
        format!("fontsize={font_px}"),
        format!("fontcolor={color}"),
        format!("x={x_px}"),
        format!("y={y_px}"),
        format!("borderw={borderw}"),
        "bordercolor=black@0.6".to_string(),
    ];

    if let Some(bg_hex) = &text.background {
        let (br, bg, bb, ba) = parse_hex_rgba(bg_hex);
        let bg_color = format!("0x{:02X}{:02X}{:02X}@{:.3}", br, bg, bb, ba as f64 / 255.0);
        parts.push("box=1".to_string());
        parts.push(format!("boxcolor={bg_color}"));
        parts.push("boxborderw=8".to_string());
    }

    if let Some(font_family) = &text.font_family {
        // ffmpeg drawtext takes a literal font file, not a family name.
        // We pass family as `font=...` which works on Windows fontconfig
        // builds; on other platforms it's ignored gracefully.
        let safe = font_family.replace('\'', "");
        parts.push(format!("font='{safe}'"));
    }

    format!("drawtext={}", parts.join(":"))
}
