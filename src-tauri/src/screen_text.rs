//! On-screen text as a compact layer for the narration prompt.
//!
//! The reference insight is that a text layer, not pixels, should carry most of
//! what the model reasons about. For talking-head footage that text comes from
//! speech; for our dominant input — a silent screen recording — it is the text
//! already on screen: window titles, terminal output, code, menu labels, error
//! dialogs.
//!
//! `build_user_message` already instructs the model to "read any text visible in
//! terminals, code editors, browsers, or dialogs". A 1024 px downscaled JPEG
//! frequently cannot satisfy that. OCR over the full-resolution frames on disk
//! makes it reliable instead of aspirational, and the result is what lets
//! narration say "runs `pnpm tauri dev`" instead of "runs a command".
//!
//! ## Structure
//!
//! Everything here is a pure function over already-recognized text except
//! [`OcrBackend`], which is the one platform-specific seam. That split is
//! deliberate: the interesting decisions — what counts as chrome, when two frames
//! say the same thing, how to spend a token budget — are testable without any
//! OCR engine, and they are where the quality of this layer actually comes from.

use crate::models::Frame;
use serde::{Deserialize, Serialize};

/// A line of text recognized on one frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextLine {
    pub text: String,
    /// Recognition confidence in 0..1, when the engine reports one.
    pub confidence: f32,
}

/// Everything recognized on a single frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameText {
    pub frame_index: usize,
    pub timestamp_seconds: f64,
    pub lines: Vec<TextLine>,
}

/// Recognition confidence below which a line is discarded.
///
/// Low-confidence output on a screen recording is usually an anti-aliased UI
/// edge read as punctuation. Feeding that to the model invites it to narrate
/// noise as if it were content.
pub const MIN_CONFIDENCE: f32 = 0.4;

/// A line appearing in more than this fraction of frames is treated as chrome.
///
/// Menu bars, window titles, tab strips, the clock and the dock persist across
/// the whole recording. They are furniture, not content, and the prompt already
/// tells the model not to narrate them — including them here would work against
/// that.
pub const CHROME_FREQUENCY: f64 = 0.8;

/// Similarity at or above which two consecutive frames are "the same screen".
///
/// Token-set Jaccard rather than string equality, because a blinking cursor or a
/// ticking clock changes a couple of tokens without changing what is on screen.
pub const DUPLICATE_SIMILARITY: f64 = 0.9;

/// Byte budget for the packed block.
///
/// Bounded because the prompt has to hold the style, the word budget, the frames
/// and any context documents too. ~10 KB is a few hundred lines of screen text —
/// far more signal than the model needs, and small next to the images.
pub const MAX_PACK_BYTES: usize = 10_000;

/// Shortest line worth keeping, in characters.
///
/// One- and two-character fragments are almost always icon labels or
/// misrecognized borders.
const MIN_LINE_CHARS: usize = 3;

/// One platform's text recognizer.
///
/// The default implementation recognizes nothing, which makes the whole layer
/// inert rather than broken on a platform without a backend — the pipeline still
/// runs, produces an empty pack, and generation proceeds exactly as it does
/// today.
pub trait OcrBackend: Send + Sync {
    /// Recognize text in the image at `path`.
    ///
    /// Returning an empty vec must mean "nothing readable", not "failed" — a
    /// backend that errors should log and return empty, because one unreadable
    /// frame is not a reason to fail a generation.
    fn recognize(&self, path: &std::path::Path) -> Vec<TextLine>;

    /// Human-readable backend name, for logs.
    fn name(&self) -> &'static str;
}

/// The fallback backend: recognizes nothing.
///
/// `dead_code` is allowed because on macOS `platform_backend` never constructs
/// this — but it is the real backend on every other target, and the tests use it
/// to pin the "layer is inert without a recognizer" contract.
#[allow(dead_code)]
pub struct NoOpBackend;

impl OcrBackend for NoOpBackend {
    fn recognize(&self, _path: &std::path::Path) -> Vec<TextLine> {
        Vec::new()
    }
    fn name(&self) -> &'static str {
        "none"
    }
}

/// Normalize a recognized line for comparison and storage.
///
/// Collapses whitespace and trims. Returns `None` for lines not worth keeping:
/// too short, too low-confidence, or with no alphanumeric content at all (a row
/// of box-drawing characters read as `|||`).
pub fn normalize_line(line: &TextLine) -> Option<String> {
    if line.confidence < MIN_CONFIDENCE {
        return None;
    }
    let collapsed = line.text.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim();
    if trimmed.chars().count() < MIN_LINE_CHARS {
        return None;
    }
    if !trimmed.chars().any(|c| c.is_alphanumeric()) {
        return None;
    }
    Some(trimmed.to_string())
}

/// Lines that appear in at least `CHROME_FREQUENCY` of frames.
///
/// Computed across the whole video rather than per-frame, because that is the
/// only way to tell a persistent menu bar from a line that happens to be on
/// screen right now.
pub fn detect_chrome(frames: &[FrameText]) -> std::collections::HashSet<String> {
    use std::collections::HashMap;

    if frames.len() < 3 {
        // With very few frames, "appears in most of them" is meaningless — a
        // 2-frame video would classify half its content as chrome.
        return Default::default();
    }

    let mut counts: HashMap<String, usize> = HashMap::new();
    for frame in frames {
        // Count each distinct line once per frame, so a line repeated within one
        // frame doesn't inflate its frequency.
        let distinct: std::collections::HashSet<String> =
            frame.lines.iter().filter_map(normalize_line).collect();
        for line in distinct {
            *counts.entry(line).or_default() += 1;
        }
    }

    let threshold = (frames.len() as f64 * CHROME_FREQUENCY).ceil() as usize;
    counts
        .into_iter()
        .filter(|(_, count)| *count >= threshold)
        .map(|(line, _)| line)
        .collect()
}

/// Token-set Jaccard similarity of two line sets, in 0..1.
fn similarity(a: &[String], b: &[String]) -> f64 {
    use std::collections::HashSet;
    let a: HashSet<&str> = a.iter().flat_map(|l| l.split_whitespace()).collect();
    let b: HashSet<&str> = b.iter().flat_map(|l| l.split_whitespace()).collect();
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let intersection = a.intersection(&b).count() as f64;
    let union = a.union(&b).count() as f64;
    if union == 0.0 {
        return 1.0;
    }
    intersection / union
}

/// One entry in the packed text layer: a screen state and when it was visible.
#[derive(Debug, Clone, PartialEq)]
pub struct ScreenState {
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub lines: Vec<String>,
}

/// Collapse per-frame text into distinct screen states.
///
/// Consecutive frames whose text is `DUPLICATE_SIMILARITY`-similar become one
/// state with a time range, so a static editor held for a minute costs one entry
/// instead of thirty. Chrome lines are removed first, since a screen that differs
/// only in its clock is not a new screen.
pub fn collapse_states(
    frames: &[FrameText],
    chrome: &std::collections::HashSet<String>,
) -> Vec<ScreenState> {
    let mut states: Vec<ScreenState> = Vec::new();

    for frame in frames {
        let lines: Vec<String> = frame
            .lines
            .iter()
            .filter_map(normalize_line)
            .filter(|l| !chrome.contains(l))
            .collect();

        if lines.is_empty() {
            continue;
        }

        match states.last_mut() {
            Some(prev) if similarity(&prev.lines, &lines) >= DUPLICATE_SIMILARITY => {
                // Same screen — extend its window rather than adding an entry.
                prev.end_seconds = frame.timestamp_seconds;
            }
            _ => states.push(ScreenState {
                start_seconds: frame.timestamp_seconds,
                end_seconds: frame.timestamp_seconds,
                lines,
            }),
        }
    }

    states
}

/// Render screen states as the prompt block, within `MAX_PACK_BYTES`.
///
/// Over budget, the *shortest* states are dropped first: a state with one line is
/// usually a transient toast, while a long one is a terminal or an editor full of
/// the content the narration should be about.
pub fn pack(states: &[ScreenState]) -> String {
    if states.is_empty() {
        return String::new();
    }

    // Rank by information content, keep as many as fit, then restore time order.
    let mut ranked: Vec<&ScreenState> = states.iter().collect();
    ranked.sort_by_key(|s| std::cmp::Reverse(s.lines.iter().map(|l| l.len()).sum::<usize>()));

    let mut kept: Vec<&ScreenState> = Vec::new();
    let mut budget = MAX_PACK_BYTES;
    for state in ranked {
        let rendered = render_state(state);
        if rendered.len() > budget {
            continue;
        }
        budget -= rendered.len();
        kept.push(state);
    }
    if kept.is_empty() {
        return String::new();
    }
    kept.sort_by(|a, b| a.start_seconds.total_cmp(&b.start_seconds));

    let body: String = kept.iter().map(|s| render_state(s)).collect();
    let dropped = states.len() - kept.len();
    let note = if dropped > 0 {
        format!(" ({dropped} shorter screens omitted for space)")
    } else {
        String::new()
    };

    format!(
        "\n\n## ON-SCREEN TEXT\n\n\
         Text read directly from the video's own frames, grouped into distinct \
         screens with the time range each was visible{note}. Use it for \
         specificity — name the actual command, file, error, or value rather \
         than describing it generically. Do NOT read it aloud verbatim; the \
         viewer can already see it.\n\n{body}"
    )
}

fn render_state(state: &ScreenState) -> String {
    format!(
        "[{:.1}-{:.1}s]\n{}\n\n",
        state.start_seconds,
        state.end_seconds,
        state
            .lines
            .iter()
            .map(|l| format!("  {l}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

/// What the OCR pass produced, so callers can report it to the user.
///
/// Returned rather than only logged: "is OCR actually on?" is not answerable
/// from the UI otherwise, and a silently-empty layer is indistinguishable from a
/// disabled one.
#[derive(Debug, Clone, Default)]
pub struct TextLayer {
    /// The prompt block. Empty when nothing usable was recognized.
    pub block: String,
    /// Distinct screens after chrome filtering and duplicate collapsing. This is
    /// the number the UI reports, because it is what the model actually receives
    /// — raw line counts are inflated by chrome that gets filtered out.
    pub screens: usize,
}

/// Run OCR over `frames` and produce the prompt block.
///
/// Recognition is parallel across frames (pure CPU, and the caller is inside
/// `spawn_blocking`). An empty `block` covers the no-backend and the
/// no-text-on-screen cases alike — in both, generation proceeds exactly as it
/// does without this layer.
pub fn build_text_layer(frames: &[Frame], backend: &dyn OcrBackend) -> TextLayer {
    use rayon::prelude::*;

    if frames.is_empty() {
        return TextLayer::default();
    }

    let recognized: Vec<FrameText> = frames
        .par_iter()
        .map(|frame| FrameText {
            frame_index: frame.index,
            timestamp_seconds: frame.timestamp_seconds,
            lines: backend.recognize(&frame.path),
        })
        .collect();

    let total_lines: usize = recognized.iter().map(|f| f.lines.len()).sum();
    if total_lines == 0 {
        tracing::debug!(
            "on-screen text: nothing recognized (backend: {})",
            backend.name()
        );
        return TextLayer::default();
    }

    let chrome = detect_chrome(&recognized);
    let states = collapse_states(&recognized, &chrome);
    let block = pack(&states);

    tracing::info!(
        "on-screen text: {} lines over {} frames → {} screens, {} chrome lines filtered, {} bytes",
        total_lines,
        frames.len(),
        states.len(),
        chrome.len(),
        block.len()
    );
    TextLayer {
        block,
        screens: states.len(),
    }
}

/// The backend for this platform.
///
/// macOS uses the Vision framework, which ships with the OS — nothing is
/// bundled and there is no model to download. Other platforms get
/// [`NoOpBackend`], which makes the layer inert rather than broken: the pipeline
/// runs, produces an empty pack, and generation proceeds exactly as it does
/// today.
pub fn platform_backend() -> Box<dyn OcrBackend> {
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::VisionBackend)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(NoOpBackend)
    }
}

/// Text recognition via the macOS Vision framework.
///
/// Vision wants a `CGImage`. There is no `CGImageSource` binding available, so
/// rather than pull in ImageIO the frame is decoded with the `image` crate — which
/// this app already uses everywhere — and wrapped as a CGImage over the decoded
/// RGBA buffer. That also means the OCR input is the full-resolution frame on
/// disk, not the 1024 px copy sent to the model, which is the point: small text is
/// exactly what this layer exists to recover.
#[cfg(target_os = "macos")]
mod macos {
    use super::{OcrBackend, TextLine};
    use objc2::rc::Retained;
    use objc2::AnyThread;
    use objc2_core_foundation::CFData;
    use objc2_core_graphics::{
        CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage,
        CGImageAlphaInfo,
    };
    use objc2_foundation::{NSArray, NSDictionary};
    use objc2_vision::{
        VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel,
    };

    pub struct VisionBackend;

    impl OcrBackend for VisionBackend {
        fn recognize(&self, path: &std::path::Path) -> Vec<TextLine> {
            let img = match image::open(path) {
                Ok(i) => i.to_rgba8(),
                Err(e) => {
                    tracing::debug!("OCR: could not decode {}: {e}", path.display());
                    return Vec::new();
                }
            };
            let (width, height) = (img.width() as usize, img.height() as usize);
            if width == 0 || height == 0 {
                return Vec::new();
            }
            let bytes = img.into_raw();

            // SAFETY: every pointer below comes from a live Retained/CF object
            // held for the duration of the call. `bytes` is copied into the
            // CFData, so the CGImage does not borrow a Rust buffer that could
            // outlive it. Row stride is exactly `width * 4` because `to_rgba8`
            // produces a tightly packed buffer.
            unsafe {
                let data = CFData::from_bytes(&bytes);
                let provider = CGDataProvider::with_cf_data(Some(&data));
                let color_space = CGColorSpace::new_device_rgb();
                let Some(cg_image) = CGImage::new(
                    width,
                    height,
                    8,
                    32,
                    width * 4,
                    color_space.as_deref(),
                    CGBitmapInfo(CGImageAlphaInfo::PremultipliedLast.0),
                    provider.as_deref(),
                    std::ptr::null(),
                    false,
                    CGColorRenderingIntent::RenderingIntentDefault,
                ) else {
                    tracing::warn!("OCR: CGImage construction failed for {}", path.display());
                    return Vec::new();
                };

                let request = VNRecognizeTextRequest::new();
                // `Accurate` over `Fast`: this runs once per frame off the hot
                // path, and small UI text is the whole point.
                request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
                // Language correction "fixes" code and CLI flags into prose —
                // `--no-verify` becomes "no verify". Off, so identifiers survive.
                request.setUsesLanguageCorrection(false);

                let handler = VNImageRequestHandler::initWithCGImage_options(
                    VNImageRequestHandler::alloc(),
                    &cg_image,
                    &NSDictionary::new(),
                );

                let as_request: Retained<VNRequest> = Retained::cast_unchecked(request.clone());
                let requests: Retained<NSArray<VNRequest>> = NSArray::from_slice(&[&*as_request]);
                if let Err(e) = handler.performRequests_error(&requests) {
                    tracing::warn!("OCR: Vision request failed on {}: {e:?}", path.display());
                    return Vec::new();
                }

                let Some(results) = request.results() else {
                    return Vec::new();
                };
                results
                    .iter()
                    .filter_map(|observation| {
                        let candidates = observation.topCandidates(1);
                        candidates.iter().next().map(|best| TextLine {
                            text: best.string().to_string(),
                            confidence: best.confidence(),
                        })
                    })
                    .collect()
            }
        }

        fn name(&self) -> &'static str {
            "macos-vision"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn line(text: &str) -> TextLine {
        TextLine {
            text: text.to_string(),
            confidence: 0.95,
        }
    }

    fn frame_text(index: usize, ts: f64, lines: &[&str]) -> FrameText {
        FrameText {
            frame_index: index,
            timestamp_seconds: ts,
            lines: lines.iter().map(|l| line(l)).collect(),
        }
    }

    // ── Line normalization ──────────────────────────────────────────────

    #[test]
    fn normalization_collapses_whitespace() {
        assert_eq!(
            normalize_line(&line("  pnpm    tauri\tdev ")).unwrap(),
            "pnpm tauri dev"
        );
    }

    #[test]
    fn low_confidence_lines_are_discarded() {
        // Anti-aliased UI edges read as punctuation with low confidence; feeding
        // them to the model invites narrating noise as content.
        let weak = TextLine {
            text: "cargo build".into(),
            confidence: 0.1,
        };
        assert!(normalize_line(&weak).is_none());
    }

    #[test]
    fn fragments_and_pure_punctuation_are_discarded() {
        for junk in ["", " ", "x", "ab", "|||", "---", "::", "  •  "] {
            assert!(
                normalize_line(&line(junk)).is_none(),
                "{junk:?} should be dropped"
            );
        }
        // But a short real token is kept.
        assert_eq!(normalize_line(&line("cd ..")).unwrap(), "cd ..");
    }

    // ── Chrome detection ────────────────────────────────────────────────

    #[test]
    fn persistent_lines_are_detected_as_chrome() {
        // A menu bar in every frame; content that changes.
        let frames = vec![
            frame_text(0, 0.0, &["File Edit View", "step one"]),
            frame_text(1, 2.0, &["File Edit View", "step two"]),
            frame_text(2, 4.0, &["File Edit View", "step three"]),
            frame_text(3, 6.0, &["File Edit View", "step four"]),
        ];
        let chrome = detect_chrome(&frames);
        assert!(chrome.contains("File Edit View"), "{chrome:?}");
        assert!(!chrome.contains("step one"), "content must survive");
    }

    #[test]
    fn a_line_in_most_but_not_all_frames_is_still_chrome() {
        // A sidebar that scrolls off once is still furniture.
        let mut frames: Vec<FrameText> = (0..10)
            .map(|i| frame_text(i, i as f64, &["Explorer", &format!("file{i}.rs")]))
            .collect();
        frames[3] = frame_text(3, 3.0, &["file3.rs"]);
        let chrome = detect_chrome(&frames);
        assert!(chrome.contains("Explorer"), "9/10 frames is chrome");
    }

    #[test]
    fn content_appearing_in_half_the_frames_is_not_chrome() {
        let frames: Vec<FrameText> = (0..10)
            .map(|i| {
                if i < 5 {
                    frame_text(i, i as f64, &["first half content"])
                } else {
                    frame_text(i, i as f64, &["second half content"])
                }
            })
            .collect();
        let chrome = detect_chrome(&frames);
        assert!(chrome.is_empty(), "50% is content, not chrome: {chrome:?}");
    }

    #[test]
    fn chrome_detection_is_skipped_for_very_few_frames() {
        // With 2 frames, "in most of them" would classify content as chrome.
        let frames = vec![
            frame_text(0, 0.0, &["some content here"]),
            frame_text(1, 1.0, &["some content here"]),
        ];
        assert!(detect_chrome(&frames).is_empty());
    }

    #[test]
    fn a_line_repeated_within_one_frame_does_not_inflate_its_frequency() {
        // Same string twice on one screen must count once, or a duplicated
        // label would look persistent.
        let frames = vec![
            frame_text(0, 0.0, &["dup", "dup", "dup", "unique a"]),
            frame_text(1, 1.0, &["unique b"]),
            frame_text(2, 2.0, &["unique c"]),
            frame_text(3, 3.0, &["unique d"]),
        ];
        assert!(!detect_chrome(&frames).contains("dup"));
    }

    // ── State collapsing ────────────────────────────────────────────────

    #[test]
    fn identical_consecutive_frames_collapse_into_one_state() {
        // A static editor held for a minute must cost one entry, not thirty.
        let frames: Vec<FrameText> = (0..10)
            .map(|i| {
                frame_text(
                    i,
                    i as f64 * 2.0,
                    &["fn main() {", "    println!(\"hi\");", "}"],
                )
            })
            .collect();
        let states = collapse_states(&frames, &HashSet::new());
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].start_seconds, 0.0);
        assert_eq!(states[0].end_seconds, 18.0, "window spans all frames");
    }

    #[test]
    fn a_changed_screen_starts_a_new_state() {
        let frames = vec![
            frame_text(0, 0.0, &["first screen content here"]),
            frame_text(1, 2.0, &["first screen content here"]),
            frame_text(2, 4.0, &["completely different output now"]),
        ];
        let states = collapse_states(&frames, &HashSet::new());
        assert_eq!(states.len(), 2);
        assert_eq!(states[1].start_seconds, 4.0);
    }

    #[test]
    fn a_ticking_clock_does_not_split_a_text_dense_screen() {
        // One changed token out of many is the same screen — this is why
        // similarity is token-set Jaccard rather than string equality. A real
        // terminal or editor frame carries dozens of tokens, so a clock tick or
        // a blinking cursor moves similarity by a couple of percent.
        let body = "Compiling narrator v0 9 5 Finished dev profile unoptimized \
                    plus debuginfo target s in 6 43s Running unittests src lib rs";
        let frames = vec![
            frame_text(0, 0.0, &[body, "12:00"]),
            frame_text(1, 2.0, &[body, "12:01"]),
        ];
        let states = collapse_states(&frames, &HashSet::new());
        assert_eq!(states.len(), 1, "a clock tick is not a new screen");
    }

    #[test]
    fn a_nearly_empty_screen_is_sensitive_to_a_single_changed_token() {
        // Documenting a real limit rather than pretending it away: with only a
        // handful of tokens on screen, one change is a large fraction of the
        // token set and does split the state. Acceptable — a near-empty screen
        // costs almost nothing to list twice, and being too eager to merge
        // would collapse genuinely different screens.
        let frames = vec![
            frame_text(0, 0.0, &["step one", "12:00"]),
            frame_text(1, 2.0, &["step one", "12:01"]),
        ];
        let states = collapse_states(&frames, &HashSet::new());
        assert_eq!(states.len(), 2);
    }

    #[test]
    fn chrome_is_removed_before_comparison() {
        let chrome: HashSet<String> = ["File Edit View".to_string()].into_iter().collect();
        let frames = vec![
            frame_text(0, 0.0, &["File Edit View", "the actual content"]),
            frame_text(1, 2.0, &["File Edit View", "the actual content"]),
        ];
        let states = collapse_states(&frames, &chrome);
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].lines, vec!["the actual content"]);
    }

    #[test]
    fn frames_with_no_usable_text_are_skipped() {
        let chrome: HashSet<String> = ["only chrome".to_string()].into_iter().collect();
        let frames = vec![
            frame_text(0, 0.0, &["only chrome"]),
            frame_text(1, 2.0, &["real content appears"]),
        ];
        let states = collapse_states(&frames, &chrome);
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].start_seconds, 2.0);
    }

    // ── Packing ─────────────────────────────────────────────────────────

    #[test]
    fn pack_is_empty_when_there_is_nothing_to_say() {
        // The no-regression case: prompt unchanged when OCR found nothing.
        assert!(pack(&[]).is_empty());
    }

    #[test]
    fn pack_includes_time_ranges_and_the_do_not_read_aloud_rule() {
        let states = vec![ScreenState {
            start_seconds: 1.0,
            end_seconds: 5.0,
            lines: vec!["cargo test --all".into()],
        }];
        let block = pack(&states);
        assert!(block.contains("ON-SCREEN TEXT"));
        assert!(block.contains("[1.0-5.0s]"), "{block}");
        assert!(block.contains("cargo test --all"));
        // Without this the model recites the screen instead of explaining it.
        assert!(block.contains("Do NOT read it aloud verbatim"), "{block}");
        assert!(block.contains("specificity"));
    }

    #[test]
    fn pack_respects_the_byte_budget() {
        // 500 states of ~100 bytes each would be ~50 KB unbounded.
        let states: Vec<ScreenState> = (0..500)
            .map(|i| ScreenState {
                start_seconds: i as f64,
                end_seconds: i as f64 + 1.0,
                lines: vec!["x".repeat(90)],
            })
            .collect();
        let block = pack(&states);
        assert!(
            block.len() < MAX_PACK_BYTES + 1_000,
            "pack was {} bytes",
            block.len()
        );
        assert!(block.contains("omitted for space"), "must say what was cut");
    }

    #[test]
    fn pack_drops_the_least_informative_states_first() {
        // A one-line toast is expendable; a full terminal is the point.
        let big_line = "y".repeat(4_000);
        let states = vec![
            ScreenState {
                start_seconds: 0.0,
                end_seconds: 1.0,
                lines: vec![big_line.clone()],
            },
            ScreenState {
                start_seconds: 2.0,
                end_seconds: 3.0,
                lines: vec!["tiny toast".into()],
            },
            ScreenState {
                start_seconds: 4.0,
                end_seconds: 5.0,
                lines: vec![big_line.clone()],
            },
            ScreenState {
                start_seconds: 6.0,
                end_seconds: 7.0,
                lines: vec![big_line],
            },
        ];
        let block = pack(&states);
        // Three 4 KB states cannot all fit in 10 KB; the toast should survive
        // only if there is room, and the big ones are prioritised.
        assert!(block.contains("yyy"), "informative states must be kept");
    }

    #[test]
    fn pack_keeps_time_order_after_ranking_by_size() {
        let states = vec![
            ScreenState {
                start_seconds: 10.0,
                end_seconds: 11.0,
                lines: vec!["short".into()],
            },
            ScreenState {
                start_seconds: 20.0,
                end_seconds: 21.0,
                lines: vec!["a much longer line of content".into()],
            },
        ];
        let block = pack(&states);
        let first = block.find("[10.0").expect("first state present");
        let second = block.find("[20.0").expect("second state present");
        assert!(first < second, "output must be chronological");
    }

    // ── End-to-end over a stub backend ──────────────────────────────────

    /// Recognizes text keyed off the file name, so the pipeline can be exercised
    /// without any OCR engine.
    struct StubBackend;

    impl OcrBackend for StubBackend {
        fn recognize(&self, path: &std::path::Path) -> Vec<TextLine> {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            // Every frame shows a menu bar; content varies by frame.
            vec![
                line("File Edit View Help"),
                line(&format!("content for {stem}")),
            ]
        }
        fn name(&self) -> &'static str {
            "stub"
        }
    }

    fn frames(n: usize) -> Vec<Frame> {
        (0..n)
            .map(|i| Frame {
                index: i,
                timestamp_seconds: i as f64 * 2.0,
                path: std::path::PathBuf::from(format!("/tmp/frame_{i}.jpg")),
                width: 1920,
                height: 1080,
            })
            .collect()
    }

    #[test]
    fn end_to_end_filters_chrome_and_keeps_content() {
        let layer = build_text_layer(&frames(5), &StubBackend);
        let block = layer.block;
        assert!(block.contains("ON-SCREEN TEXT"));
        assert!(
            !block.contains("File Edit View Help"),
            "the persistent menu bar must be filtered as chrome:\n{block}"
        );
        assert!(block.contains("content for frame_0"), "{block}");
        assert!(block.contains("content for frame_4"), "{block}");
    }

    #[test]
    fn end_to_end_is_empty_with_the_no_op_backend() {
        // The default on any platform without a recognizer: the layer is inert
        // and the prompt is byte-identical to before this feature.
        assert!(build_text_layer(&frames(5), &NoOpBackend).block.is_empty());
        assert_eq!(NoOpBackend.name(), "none");
    }

    #[test]
    fn end_to_end_handles_no_frames() {
        assert!(build_text_layer(&[], &StubBackend).block.is_empty());
    }

    #[test]
    fn platform_backend_is_constructible() {
        // Whatever this platform provides, it must at least build and be safe to
        // call on a nonexistent path.
        let backend = platform_backend();
        assert!(!backend.name().is_empty());
        assert!(backend
            .recognize(std::path::Path::new("/nonexistent/none.jpg"))
            .is_empty());
    }

    /// A committed 460x90 PNG reading "pnpm tauri dev".
    ///
    /// A fixture rather than a render at test time: generating it needs ffmpeg
    /// with `drawtext` (libfreetype), which the ffmpeg on a given machine may not
    /// have — and a test that silently skips on the machine where it matters most
    /// is not a test. 3.5 KB.
    #[cfg(target_os = "macos")]
    const TEXT_FIXTURE: &str = "tests/fixtures/onscreen_text.png";

    /// The claim this whole feature rests on: Vision can read the text in a
    /// screen recording, so narration can name the actual command instead of
    /// describing it generically.
    #[cfg(target_os = "macos")]
    #[test]
    fn vision_backend_reads_text_from_a_screenshot() {
        let png = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(TEXT_FIXTURE);
        assert!(png.is_file(), "missing fixture at {}", png.display());

        let lines = macos::VisionBackend.recognize(&png);
        assert!(!lines.is_empty(), "Vision returned nothing");
        let joined = lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            joined.to_lowercase().contains("tauri"),
            "expected the rendered command, got: {joined:?}"
        );
        assert!(
            lines.iter().all(|l| l.confidence > 0.0),
            "confidence must be populated so MIN_CONFIDENCE can filter"
        );
    }

    /// A frame with nothing readable must yield nothing, not a spurious line —
    /// otherwise every blank screen contributes noise to the prompt.
    #[cfg(target_os = "macos")]
    #[test]
    fn vision_backend_returns_nothing_for_a_blank_image() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("blank.png");
        image::RgbImage::from_pixel(400, 200, image::Rgb([255, 255, 255]))
            .save(&png)
            .unwrap();
        let lines = macos::VisionBackend.recognize(&png);
        let usable: Vec<String> = lines.iter().filter_map(normalize_line).collect();
        assert!(usable.is_empty(), "blank image produced {usable:?}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn vision_backend_survives_unreadable_and_degenerate_input() {
        // One bad frame must cost one frame, never the generation.
        assert!(macos::VisionBackend
            .recognize(std::path::Path::new("/nonexistent/x.png"))
            .is_empty());

        let dir = tempfile::tempdir().unwrap();
        let not_an_image = dir.path().join("garbage.png");
        std::fs::write(&not_an_image, b"this is not a PNG").unwrap();
        assert!(macos::VisionBackend.recognize(&not_an_image).is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn platform_backend_on_macos_is_vision() {
        assert_eq!(platform_backend().name(), "macos-vision");
    }
}
