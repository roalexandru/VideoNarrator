import { describe, it, expect } from "vitest";
import {
  canProceedFromStep0,
  canProceedFromStep1,
  canProceedFromStep3,
} from "../lib/validation";
import type { VideoFile } from "../types/project";

const makeVideo = (overrides?: Partial<VideoFile>): VideoFile => ({
  path: "/test.mp4",
  name: "test.mp4",
  size: 1000,
  duration: 30,
  resolution: { width: 1920, height: 1080 },
  codec: "h264",
  fps: 30,
  ...overrides,
});

describe("canProceedFromStep0 — edge cases", () => {
  it("returns false when both video and title are missing", () => {
    expect(canProceedFromStep0(null, "")).toBe(false);
  });

  it("returns false when title is only whitespace (tabs/newlines)", () => {
    expect(canProceedFromStep0(makeVideo(), "\t")).toBe(false);
    expect(canProceedFromStep0(makeVideo(), "\n")).toBe(false);
    expect(canProceedFromStep0(makeVideo(), "  \t\n  ")).toBe(false);
  });

  it("returns true for a single-character title", () => {
    expect(canProceedFromStep0(makeVideo(), "A")).toBe(true);
  });

  it("returns true for a very long title", () => {
    const longTitle = "A".repeat(1000);
    expect(canProceedFromStep0(makeVideo(), longTitle)).toBe(true);
  });

  it("returns true for title with special characters", () => {
    expect(canProceedFromStep0(makeVideo(), "Video #1 — 'Test' & <Demo>")).toBe(true);
  });

  it("returns true for title with unicode characters", () => {
    expect(canProceedFromStep0(makeVideo(), "ビデオナレーション")).toBe(true);
  });

  it("returns true for title with leading/trailing spaces if non-empty after trim", () => {
    expect(canProceedFromStep0(makeVideo(), "  Hello  ")).toBe(true);
  });

  it("returns false when video is null even with valid title", () => {
    expect(canProceedFromStep0(null, "Valid Title")).toBe(false);
  });
});

describe("canProceedFromStep1 — edge cases", () => {
  it("returns false with empty array", () => {
    expect(canProceedFromStep1([])).toBe(false);
  });

  it("returns true with single language", () => {
    expect(canProceedFromStep1(["en"])).toBe(true);
  });

  it("returns true with multiple languages", () => {
    expect(canProceedFromStep1(["en", "ja", "es", "fr"])).toBe(true);
  });

  it("returns true even with empty string language (no content validation)", () => {
    // The function only checks array length, not content
    expect(canProceedFromStep1([""])).toBe(true);
  });
});

describe("canProceedFromStep3 — edge cases", () => {
  it("returns false with 0 segments", () => {
    expect(canProceedFromStep3(0)).toBe(false);
  });

  it("returns true with 1 segment", () => {
    expect(canProceedFromStep3(1)).toBe(true);
  });

  it("returns true with large number of segments", () => {
    expect(canProceedFromStep3(999)).toBe(true);
  });

  it("returns false with negative segment count", () => {
    expect(canProceedFromStep3(-1)).toBe(false);
  });
});
