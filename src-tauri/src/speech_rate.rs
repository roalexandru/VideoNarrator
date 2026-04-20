//! Per-language speech-rate budget used to decide whether a segment's text
//! will fit inside its `[start_seconds, end_seconds]` window when spoken by
//! the TTS engine. A single source of truth used by:
//!
//!   - prompt construction (budget shown to the LLM)
//!   - `script_validator::validate_speech_rate` (post-parse check)
//!   - export-time compression (atempo cap in `commands::generate_tts`)
//!   - Review UI preview (mirrored in `src/lib/speechRate.ts`)
//!
//! When changing any constant here, also update the TS mirror or the Review
//! prediction will disagree with the actual export.
//!
//! Non-CJK languages use words-per-minute; Japanese uses significant characters
//! per second because "words" aren't a meaningful unit in CJK TTS.

use serde::{Deserialize, Serialize};

/// Target words-per-minute for languages where word count is meaningful.
/// Values chosen to match typical natural TTS output (Azure Jenny, ElevenLabs
/// default voices) measured at playback speed 1.0.
const WPM_EN: f64 = 150.0;
const WPM_DE: f64 = 135.0;
const WPM_FR: f64 = 160.0;
const WPM_PT_BR: f64 = 155.0;

/// Japanese is character-based. 400 chars/min ≈ 6.67 chars/sec, which matches
/// natural Azure ja-JP voices. "Significant" = non-whitespace, non-punctuation.
const JA_CHARS_PER_SEC: f64 = 400.0 / 60.0;

/// Compression cap applied at export (see `commands::generate_tts`). Kept here
/// so the Review UI's prediction uses the exact same threshold.
pub const COMPRESSION_CAP: f64 = 1.20;

/// Return the predicted TTS duration (seconds) for a given text+language at
/// natural playback speed. Identical output to
/// `src/lib/speechRate.ts::estimateTtsSeconds`.
pub fn estimate_tts_seconds(text: &str, lang: &str) -> f64 {
    let normalized = normalize_lang(lang);
    if normalized == "ja" {
        let chars = count_cjk_significant_chars(text) as f64;
        return chars / JA_CHARS_PER_SEC;
    }
    let wpm = match normalized.as_str() {
        "de" => WPM_DE,
        "fr" => WPM_FR,
        "pt-br" => WPM_PT_BR,
        // "en" and any unknown language fall back to English — the most common
        // case and a reasonable default that errs on the generous side.
        _ => WPM_EN,
    };
    let words = count_words(text) as f64;
    60.0 * words / wpm
}

/// The maximum number of words that fit in `window_seconds` at the target rate
/// for `lang`. For Japanese, returns character count instead. Used to build the
/// per-segment word budget shown to the LLM.
#[allow(dead_code)] // exercised by tests and reserved for per-chunk prompt additions
pub fn word_budget(window_seconds: f64, lang: &str) -> usize {
    let rate_per_min = rate_per_minute(lang);
    ((window_seconds * rate_per_min / 60.0).round() as i64).max(1) as usize
}

/// Units-per-minute for budget display. Japanese returns chars-per-minute;
/// everything else returns WPM.
pub fn rate_per_minute(lang: &str) -> f64 {
    match normalize_lang(lang).as_str() {
        "ja" => JA_CHARS_PER_SEC * 60.0,
        "de" => WPM_DE,
        "fr" => WPM_FR,
        "pt-br" => WPM_PT_BR,
        _ => WPM_EN,
    }
}

/// The unit label for the budget ("words" vs "characters"). Only used in
/// prompt construction so the LLM knows what the number counts.
pub fn budget_unit(lang: &str) -> &'static str {
    if normalize_lang(lang) == "ja" {
        "characters"
    } else {
        "words"
    }
}

/// Language code normalization. Accepts `en`, `en-US`, `pt-BR`, `ja-JP` etc.
/// Returns lowercase with region only when it changes the rate (pt-BR).
fn normalize_lang(lang: &str) -> String {
    let lower = lang.to_ascii_lowercase();
    // Only pt-BR is configured today. Match the `pt` base and the `pt-*`
    // region form specifically — `starts_with("pt")` would also swallow
    // `ptolemy` or any future `ptX` code and silently classify it as
    // Brazilian Portuguese. European `pt-PT` also lands on pt-BR for now;
    // once pt-PT is tuned separately this branch is the place to split.
    if lower == "pt" || lower.starts_with("pt-") {
        return "pt-br".to_string();
    }
    // Strip any region suffix: "en-US" -> "en", "ja-JP" -> "ja".
    if let Some((base, _)) = lower.split_once('-') {
        return base.to_string();
    }
    lower
}

/// Word count for non-CJK text. Splits on unicode whitespace and discards
/// empty tokens. Punctuation attached to words ("don't", "UiPath.") counts as
/// one word — same as a human reader.
fn count_words(text: &str) -> usize {
    text.split_whitespace()
        .filter(|w| w.chars().any(|c| c.is_alphanumeric()))
        .count()
}

/// Significant CJK character count for Japanese. Counts hiragana, katakana,
/// and CJK unified ideographs. Skips whitespace and ASCII punctuation so
/// "これは、テスト。" counts 6, not 8.
fn count_cjk_significant_chars(text: &str) -> usize {
    text.chars()
        .filter(|c| {
            let cp = *c as u32;
            // Hiragana
            (0x3040..=0x309F).contains(&cp)
                // Katakana
                || (0x30A0..=0x30FF).contains(&cp)
                // CJK Unified Ideographs + common extensions
                || (0x4E00..=0x9FFF).contains(&cp)
                || (0x3400..=0x4DBF).contains(&cp)
                // Half-width katakana
                || (0xFF66..=0xFF9F).contains(&cp)
        })
        .count()
}

// ── Overflow reporting ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Predicted / window ≤ 0.90. Nothing to worry about.
    Fit,
    /// 0.90 < ratio ≤ 1.00. Will fit at natural speed but has no breathing room.
    Tight,
    /// 1.00 < ratio ≤ `COMPRESSION_CAP`. Export will speed this segment up to fit.
    Compress,
    /// ratio > `COMPRESSION_CAP`. Even after speed-up, export must extend the
    /// video with a held final frame for this segment's overflow.
    Overflow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentOverflow {
    pub index: usize,
    pub predicted_seconds: f64,
    pub window_seconds: f64,
    pub severity: Severity,
}

impl SegmentOverflow {
    pub fn from_ratio(index: usize, predicted: f64, window: f64) -> Self {
        let ratio = if window > 0.0 {
            predicted / window
        } else {
            0.0
        };
        let severity = if ratio <= 0.90 {
            Severity::Fit
        } else if ratio <= 1.00 {
            Severity::Tight
        } else if ratio <= COMPRESSION_CAP {
            Severity::Compress
        } else {
            Severity::Overflow
        };
        Self {
            index,
            predicted_seconds: predicted,
            window_seconds: window,
            severity,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_150_wpm_exact() {
        // 150 words / 150 wpm = 60 s.
        let text = "word ".repeat(150);
        let secs = estimate_tts_seconds(text.trim(), "en");
        assert!((secs - 60.0).abs() < 1e-6, "got {secs}");
    }

    #[test]
    fn english_region_suffix_treated_as_en() {
        let text = "word ".repeat(75);
        let a = estimate_tts_seconds(text.trim(), "en");
        let b = estimate_tts_seconds(text.trim(), "en-US");
        assert!((a - b).abs() < 1e-9);
    }

    #[test]
    fn german_slower_than_english() {
        let text = "wort ".repeat(135);
        let de = estimate_tts_seconds(text.trim(), "de");
        let en = estimate_tts_seconds(text.trim(), "en");
        assert!(de > en, "de={de} en={en}");
        assert!((de - 60.0).abs() < 1e-6);
    }

    #[test]
    fn french_faster_than_english() {
        let text = "mot ".repeat(160);
        let fr = estimate_tts_seconds(text.trim(), "fr");
        assert!((fr - 60.0).abs() < 1e-6);
    }

    #[test]
    fn pt_br_matches_config() {
        let text = "palavra ".repeat(155);
        let pt = estimate_tts_seconds(text.trim(), "pt-BR");
        assert!((pt - 60.0).abs() < 1e-6);
    }

    #[test]
    fn japanese_uses_chars_per_sec() {
        // 400 chars / 6.67 cps = 60s. Use hiragana 'あ' since it's one char each.
        let text = "あ".repeat(400);
        let ja = estimate_tts_seconds(&text, "ja");
        assert!((ja - 60.0).abs() < 1e-6, "got {ja}");
    }

    #[test]
    fn japanese_ignores_punctuation() {
        // "これは、テスト。" has 6 significant CJK chars + 2 punctuation.
        let secs = estimate_tts_seconds("これは、テスト。", "ja");
        let expected = 6.0 / JA_CHARS_PER_SEC;
        assert!((secs - expected).abs() < 1e-6);
    }

    #[test]
    fn word_budget_round_trips() {
        // 10s at 150 wpm = 25 words.
        assert_eq!(word_budget(10.0, "en"), 25);
        // 10s at 135 wpm = 22.5 → rounds to 23.
        assert_eq!(word_budget(10.0, "de"), 23);
    }

    #[test]
    fn severity_thresholds() {
        let fit = SegmentOverflow::from_ratio(0, 5.0, 10.0); // 0.5
        assert_eq!(fit.severity, Severity::Fit);

        let tight = SegmentOverflow::from_ratio(0, 9.5, 10.0); // 0.95
        assert_eq!(tight.severity, Severity::Tight);

        let compress = SegmentOverflow::from_ratio(0, 11.0, 10.0); // 1.1
        assert_eq!(compress.severity, Severity::Compress);

        let overflow = SegmentOverflow::from_ratio(0, 20.0, 10.0); // 2.0
        assert_eq!(overflow.severity, Severity::Overflow);
    }

    #[test]
    fn counts_punctuated_words_as_one() {
        // "UiPath." and "don't" each count as one word.
        assert_eq!(count_words("UiPath. don't stop."), 3);
    }

    #[test]
    fn empty_text_is_zero() {
        assert!(estimate_tts_seconds("", "en").abs() < 1e-9);
        assert!(estimate_tts_seconds("", "ja").abs() < 1e-9);
    }

    #[test]
    fn unknown_language_falls_back_to_english() {
        let text = "word ".repeat(150);
        let en = estimate_tts_seconds(text.trim(), "en");
        let unknown = estimate_tts_seconds(text.trim(), "xx");
        assert!((en - unknown).abs() < 1e-9);
    }
}
