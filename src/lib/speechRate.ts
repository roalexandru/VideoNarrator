// TypeScript mirror of `src-tauri/src/speech_rate.rs`. Must produce byte-identical
// numeric output to the Rust side so the Review preview agrees with what
// Export actually does. When changing a constant or the estimator formula
// here, update the Rust side in the same commit.

const WPM_EN = 150;
const WPM_DE = 135;
const WPM_FR = 160;
const WPM_PT_BR = 155;
const JA_CHARS_PER_SEC = 400 / 60;

export const COMPRESSION_CAP = 1.20;

export type Severity = "fit" | "tight" | "compress" | "overflow";

export interface SegmentOverflow {
  index: number;
  predicted_seconds: number;
  window_seconds: number;
  severity: Severity;
}

/**
 * Predicted TTS duration in seconds for `text` in the given language at
 * natural playback speed. Mirrors `speech_rate::estimate_tts_seconds`.
 *
 * `speedMultiplier` scales the predicted duration: a configured Azure/ElevenLabs
 * speed of 1.2 produces audio 20% shorter than natural, so the prediction
 * divides by 1.2. Pass 1.0 (default) for natural-pace estimates.
 */
export function estimateTtsSeconds(text: string, lang: string, speedMultiplier: number = 1.0): number {
  const safeSpeed = speedMultiplier > 0 ? speedMultiplier : 1.0;
  const normalized = normalizeLang(lang);
  if (normalized === "ja") {
    const chars = countCjkSignificantChars(text);
    return chars / JA_CHARS_PER_SEC / safeSpeed;
  }
  const wpm = wpmFor(normalized);
  const words = countWords(text);
  return (60 * words) / wpm / safeSpeed;
}

export function ratePerMinute(lang: string): number {
  const normalized = normalizeLang(lang);
  if (normalized === "ja") return JA_CHARS_PER_SEC * 60;
  return wpmFor(normalized);
}

export function budgetUnit(lang: string): "words" | "characters" {
  return normalizeLang(lang) === "ja" ? "characters" : "words";
}

/**
 * Classify a predicted-vs-window ratio into a severity bucket. Thresholds
 * match `speech_rate::SegmentOverflow::from_ratio`.
 */
export function classifySeverity(predicted: number, window: number): Severity {
  const ratio = window > 0 ? predicted / window : 0;
  if (ratio <= 0.90) return "fit";
  if (ratio <= 1.00) return "tight";
  if (ratio <= COMPRESSION_CAP) return "compress";
  return "overflow";
}

/**
 * Build a full per-segment report for a given list of segments + language.
 * Used by Review when no server-side report is attached to the script yet
 * (e.g. after a local edit).
 *
 * `speedMultiplier` matches the configured TTS speed so the predictions
 * match what Export will actually produce.
 */
export function computeSpeechRateReport(
  segments: Array<{ index: number; start_seconds: number; end_seconds: number; text: string }>,
  lang: string,
  speedMultiplier: number = 1.0,
): SegmentOverflow[] {
  return segments.map((seg) => {
    const window = Math.max(0.001, seg.end_seconds - seg.start_seconds);
    const predicted = estimateTtsSeconds(seg.text, lang, speedMultiplier);
    return {
      index: seg.index,
      predicted_seconds: predicted,
      window_seconds: window,
      severity: classifySeverity(predicted, window),
    };
  });
}

/**
 * Predict what Export will produce: how many segments need atempo speed-up,
 * how many still overflow after the cap, and how much freeze-frame padding
 * will be appended to the video. Mirrors the audio-concat loop in
 * `commands.rs::generate_tts` (compact branch) and the `pad_video_to_audio_length`
 * check in `video_edit.rs::merge_audio_video`.
 *
 * `segments` must be in the same timeline order the script will export in.
 */
export function predictExport(
  segments: Array<{ start_seconds: number; end_seconds: number; text: string }>,
  lang: string,
  videoDuration: number,
  speedMultiplier: number = 1.0,
): {
  compressed: number;
  overCap: number;
  padSeconds: number;
  /**
   * Segments whose scheduled start time is at or past the end of the video.
   * These can't be covered by frames at all — at export, the last video frame
   * is held while their narration plays. Typically signals a script produced
   * against a wrong (inflated) video-duration, not a merely tight segment.
   */
  segmentsPastEnd: number;
} {
  let compressed = 0;
  let overCap = 0;
  let segmentsPastEnd = 0;
  let audioPos = 0;

  for (const seg of segments) {
    const window = Math.max(0.5, seg.end_seconds - seg.start_seconds);
    const predicted = estimateTtsSeconds(seg.text, lang, speedMultiplier);

    if (videoDuration > 0 && seg.start_seconds >= videoDuration - 0.5) {
      segmentsPastEnd += 1;
    }

    // Silence gap before the segment, matching the Rust loop's `if gap > 0.05`
    // guard. A negative gap (previous segment overran) is dropped.
    const gap = seg.start_seconds - audioPos;
    if (gap > 0.05) {
      audioPos += gap;
    }

    let effective: number;
    if (predicted > window + 0.1) {
      const idealSpeed = predicted / window;
      const appliedSpeed = Math.min(idealSpeed, COMPRESSION_CAP);
      effective = predicted / appliedSpeed;
      compressed += 1;
      if (idealSpeed > COMPRESSION_CAP) {
        overCap += 1;
      }
    } else {
      effective = predicted;
    }
    audioPos += effective;
  }

  // Rust adds trailing silence up to video duration, but only if the last
  // scheduled `end` is still short of the video. That silence never makes
  // the audio longer than the video — only the `audioPos > videoDuration`
  // case does.
  const padSeconds = Math.max(0, audioPos - videoDuration);
  return { compressed, overCap, padSeconds, segmentsPastEnd };
}

// ── private helpers ────────────────────────────────────────────────────────

function normalizeLang(lang: string): string {
  const lower = lang.toLowerCase();
  // Match `pt` and `pt-*` specifically — plain `startsWith("pt")` would
  // also swallow `ptolemy`-style codes and classify them as Portuguese.
  // Mirrors `speech_rate::normalize_lang` on the Rust side.
  if (lower === "pt" || lower.startsWith("pt-")) return "pt-br";
  const dash = lower.indexOf("-");
  return dash >= 0 ? lower.slice(0, dash) : lower;
}

function wpmFor(normalizedLang: string): number {
  switch (normalizedLang) {
    case "de":
      return WPM_DE;
    case "fr":
      return WPM_FR;
    case "pt-br":
      return WPM_PT_BR;
    default:
      return WPM_EN;
  }
}

function countWords(text: string): number {
  return text
    .split(/\s+/)
    .filter((w) => w.length > 0 && /[\p{L}\p{N}]/u.test(w))
    .length;
}

function countCjkSignificantChars(text: string): number {
  let n = 0;
  for (const ch of text) {
    const cp = ch.codePointAt(0);
    if (cp === undefined) continue;
    if (
      (cp >= 0x3040 && cp <= 0x309F) ||
      (cp >= 0x30A0 && cp <= 0x30FF) ||
      (cp >= 0x4E00 && cp <= 0x9FFF) ||
      (cp >= 0x3400 && cp <= 0x4DBF) ||
      (cp >= 0xFF66 && cp <= 0xFF9F)
    ) {
      n += 1;
    }
  }
  return n;
}
