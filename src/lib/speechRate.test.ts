import { describe, it, expect } from "vitest";
import {
  estimateTtsSeconds,
  classifySeverity,
  computeSpeechRateReport,
  predictExport,
  ratePerMinute,
  budgetUnit,
  COMPRESSION_CAP,
} from "./speechRate";

// These numeric expectations must match the Rust tests in
// `src-tauri/src/speech_rate.rs`. When adjusting a constant or the estimator,
// update both sides in the same commit — otherwise Review's prediction will
// diverge from what Export actually produces.

describe("speechRate.estimateTtsSeconds", () => {
  it("English 150 wpm: 150 words = 60 seconds exactly", () => {
    const text = "word ".repeat(150).trim();
    expect(estimateTtsSeconds(text, "en")).toBeCloseTo(60, 9);
  });

  it("Region suffix en-US is treated as en", () => {
    const text = "word ".repeat(75).trim();
    expect(estimateTtsSeconds(text, "en")).toBeCloseTo(estimateTtsSeconds(text, "en-US"), 9);
  });

  it("German is slower than English (135 wpm)", () => {
    const text = "wort ".repeat(135).trim();
    expect(estimateTtsSeconds(text, "de")).toBeCloseTo(60, 9);
  });

  it("French is faster than English (160 wpm)", () => {
    const text = "mot ".repeat(160).trim();
    expect(estimateTtsSeconds(text, "fr")).toBeCloseTo(60, 9);
  });

  it("Portuguese-BR uses its own rate", () => {
    const text = "palavra ".repeat(155).trim();
    expect(estimateTtsSeconds(text, "pt-BR")).toBeCloseTo(60, 9);
  });

  it("Japanese counts chars at 400/min", () => {
    const text = "あ".repeat(400);
    expect(estimateTtsSeconds(text, "ja")).toBeCloseTo(60, 9);
  });

  it("Japanese ignores punctuation", () => {
    // 6 significant CJK chars + 2 punctuation
    expect(estimateTtsSeconds("これは、テスト。", "ja")).toBeCloseTo(6 / (400 / 60), 9);
  });

  it("Empty text is zero", () => {
    expect(estimateTtsSeconds("", "en")).toBe(0);
    expect(estimateTtsSeconds("", "ja")).toBe(0);
  });

  it("Unknown language falls back to English", () => {
    const text = "word ".repeat(150).trim();
    expect(estimateTtsSeconds(text, "xx")).toBeCloseTo(estimateTtsSeconds(text, "en"), 9);
  });
});

describe("speechRate.estimateTtsSeconds — speed multiplier", () => {
  it("speed=1.0 default matches natural-pace estimate", () => {
    const text = "word ".repeat(150).trim();
    expect(estimateTtsSeconds(text, "en")).toBeCloseTo(60, 9);
    expect(estimateTtsSeconds(text, "en", 1.0)).toBeCloseTo(60, 9);
  });

  it("speed=1.2 produces 20% shorter audio (Azure-like)", () => {
    const text = "word ".repeat(150).trim();
    expect(estimateTtsSeconds(text, "en", 1.2)).toBeCloseTo(50, 6);
  });

  it("speed=0.8 produces 25% longer audio (slowed TTS)", () => {
    const text = "word ".repeat(120).trim();
    // 120 words / 150 wpm = 48s, / 0.8 = 60s
    expect(estimateTtsSeconds(text, "en", 0.8)).toBeCloseTo(60, 6);
  });

  it("invalid speed falls back to 1.0", () => {
    const text = "word ".repeat(150).trim();
    expect(estimateTtsSeconds(text, "en", 0)).toBeCloseTo(60, 9);
    expect(estimateTtsSeconds(text, "en", -1)).toBeCloseTo(60, 9);
  });

  it("Japanese also respects speed multiplier", () => {
    const text = "あ".repeat(400);
    expect(estimateTtsSeconds(text, "ja", 2.0)).toBeCloseTo(30, 6);
  });
});

describe("speechRate.predictExport — speed multiplier", () => {
  it("Azure speed=1.5 turns an overflow segment into a fit", () => {
    // 20 words / 150 wpm = 8s natural; with speed 1.5 → 5.33s. Fits 10s window.
    const segments = [
      { start_seconds: 0, end_seconds: 10, text: "word ".repeat(20).trim() },
    ];
    const atNatural = predictExport(segments, "en", 10, 1.0);
    const atFast = predictExport(segments, "en", 10, 1.5);
    expect(atNatural.compressed).toBe(0); // 8s fits 10s window at 1.0
    expect(atFast.compressed).toBe(0);    // still fits at 1.5
    expect(atNatural.padSeconds).toBe(0);
    expect(atFast.padSeconds).toBe(0);
  });

  it("Slower speed=0.8 can push a tight fit into compression", () => {
    // 24 words / 150 wpm = 9.6s → tight at 1.0 (fits 10s); at 0.8 → 12s (overflow).
    const segments = [
      { start_seconds: 0, end_seconds: 10, text: "word ".repeat(24).trim() },
    ];
    const atNatural = predictExport(segments, "en", 20, 1.0);
    const atSlow = predictExport(segments, "en", 20, 0.8);
    expect(atNatural.compressed).toBe(0);
    expect(atSlow.compressed).toBe(1);
  });
});

describe("speechRate.classifySeverity", () => {
  it("ratio <= 0.90 is fit", () => {
    expect(classifySeverity(5, 10)).toBe("fit");
    expect(classifySeverity(9, 10)).toBe("fit");
  });

  it("0.90 < ratio <= 1.00 is tight", () => {
    expect(classifySeverity(9.5, 10)).toBe("tight");
    expect(classifySeverity(10, 10)).toBe("tight");
  });

  it("1.00 < ratio <= COMPRESSION_CAP is compress", () => {
    expect(classifySeverity(11, 10)).toBe("compress");
    expect(classifySeverity(COMPRESSION_CAP * 10, 10)).toBe("compress");
  });

  it("ratio > COMPRESSION_CAP is overflow", () => {
    expect(classifySeverity(COMPRESSION_CAP * 10 + 0.001, 10)).toBe("overflow");
    expect(classifySeverity(20, 10)).toBe("overflow");
  });
});

describe("speechRate.ratePerMinute / budgetUnit", () => {
  it("Japanese returns chars-per-minute and 'characters' unit", () => {
    expect(ratePerMinute("ja")).toBeCloseTo(400, 9);
    expect(budgetUnit("ja")).toBe("characters");
  });

  it("Other languages return WPM and 'words'", () => {
    expect(ratePerMinute("en")).toBe(150);
    expect(ratePerMinute("de")).toBe(135);
    expect(budgetUnit("en")).toBe("words");
  });
});

describe("speechRate.computeSpeechRateReport", () => {
  it("Maps each segment to an overflow entry with its severity", () => {
    const dense = "word ".repeat(50).trim(); // ~20s at 150 wpm
    const short = "hello world";              // ~0.8s
    const report = computeSpeechRateReport(
      [
        { index: 0, start_seconds: 0,  end_seconds: 10, text: dense },
        { index: 1, start_seconds: 10, end_seconds: 20, text: short },
      ],
      "en",
    );
    expect(report).toHaveLength(2);
    expect(report[0].severity).toBe("overflow");
    expect(report[1].severity).toBe("fit");
    expect(report[0].predicted_seconds).toBeCloseTo(20, 9);
  });
});

describe("speechRate.predictExport", () => {
  it("All-fit script predicts zero compression and zero padding", () => {
    const segments = [
      { start_seconds: 0,  end_seconds: 10, text: "hello world" },
      { start_seconds: 10, end_seconds: 20, text: "another short line" },
    ];
    const p = predictExport(segments, "en", 20);
    expect(p.compressed).toBe(0);
    expect(p.overCap).toBe(0);
    expect(p.padSeconds).toBe(0);
  });

  it("Mild overrun triggers compression but no padding when timeline absorbs it", () => {
    // 28 words in a 10s window → ~11.2s at 150 wpm → ratio 1.12 → Compress.
    const segments = [
      { start_seconds: 0, end_seconds: 10, text: "word ".repeat(28).trim() },
    ];
    const p = predictExport(segments, "en", 20);
    expect(p.compressed).toBe(1);
    expect(p.overCap).toBe(0);
    expect(p.padSeconds).toBe(0);
  });

  it("Severe overrun flags over-cap and predicts freeze-frame padding", () => {
    // 100 words in a 10s window → ~40s → ratio 4.0, cap 1.20 → 33.3s effective.
    // Video is only 10s long → padding ~23.3s.
    const segments = [
      { start_seconds: 0, end_seconds: 10, text: "word ".repeat(100).trim() },
    ];
    const p = predictExport(segments, "en", 10);
    expect(p.compressed).toBe(1);
    expect(p.overCap).toBe(1);
    expect(p.padSeconds).toBeGreaterThan(20);
  });

  it("Mirrors Prem's repro: 100 words in 10.8s window is the red-flag case", () => {
    // Simulates the failing segment from the Autopilot project.
    const segments = [
      { start_seconds: 94.0, end_seconds: 104.8, text: "word ".repeat(100).trim() },
    ];
    const p = predictExport(segments, "en", 104.8);
    expect(p.overCap).toBe(1); // cap at 1.20× can't save this
    expect(p.padSeconds).toBeGreaterThan(0);
  });
});
