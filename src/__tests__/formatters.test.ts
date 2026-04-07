import { describe, it, expect } from "vitest";
import { secondsToTimestamp, formatFileSize, formatDuration } from "../lib/formatters";

describe("secondsToTimestamp — edge cases", () => {
  it("handles negative numbers", () => {
    // Math.floor of negative: -1/60 = -0.016... -> Math.floor = -1
    // The function is not designed for negatives, but should not throw
    expect(() => secondsToTimestamp(-5)).not.toThrow();
  });

  it("handles zero", () => {
    expect(secondsToTimestamp(0)).toBe("0:00");
  });

  it("handles very large values", () => {
    // 3600 seconds = 60 minutes
    expect(secondsToTimestamp(3600)).toBe("60:00");
    // 7200 seconds = 120 minutes
    expect(secondsToTimestamp(7200)).toBe("120:00");
  });

  it("handles very small positive fractions", () => {
    expect(secondsToTimestamp(0.1)).toBe("0:00");
    expect(secondsToTimestamp(0.99)).toBe("0:00");
  });

  it("handles NaN", () => {
    // NaN propagation — should not throw
    expect(() => secondsToTimestamp(NaN)).not.toThrow();
  });

  it("handles exactly 59 seconds", () => {
    expect(secondsToTimestamp(59)).toBe("0:59");
  });

  it("handles exactly 60 seconds", () => {
    expect(secondsToTimestamp(60)).toBe("1:00");
  });

  it("handles 61 seconds", () => {
    expect(secondsToTimestamp(61)).toBe("1:01");
  });
});

describe("formatFileSize — edge cases", () => {
  it("handles zero bytes", () => {
    expect(formatFileSize(0)).toBe("0 B");
  });

  it("handles negative bytes", () => {
    // Not a realistic scenario, but should not throw
    expect(() => formatFileSize(-100)).not.toThrow();
  });

  it("handles exactly 1 KB boundary (1024)", () => {
    expect(formatFileSize(1024)).toBe("1.0 KB");
  });

  it("handles exactly 1 MB boundary", () => {
    expect(formatFileSize(1024 * 1024)).toBe("1.0 MB");
  });

  it("handles exactly 1 GB boundary", () => {
    expect(formatFileSize(1024 * 1024 * 1024)).toBe("1.0 GB");
  });

  it("handles very large values (terabyte range)", () => {
    const tb = 1024 * 1024 * 1024 * 1024;
    expect(formatFileSize(tb)).toBe("1024.0 GB");
  });

  it("handles 1 byte", () => {
    expect(formatFileSize(1)).toBe("1 B");
  });

  it("handles 1023 bytes (just under KB)", () => {
    expect(formatFileSize(1023)).toBe("1023 B");
  });

  it("handles fractional KB", () => {
    expect(formatFileSize(1536)).toBe("1.5 KB");
  });

  it("handles NaN", () => {
    expect(() => formatFileSize(NaN)).not.toThrow();
  });
});

describe("formatDuration — edge cases", () => {
  it("handles zero", () => {
    expect(formatDuration(0)).toBe("0s");
  });

  it("handles exactly 60 seconds", () => {
    expect(formatDuration(60)).toBe("1m 0s");
  });

  it("handles negative numbers", () => {
    // Not a realistic scenario, but should not throw
    expect(() => formatDuration(-10)).not.toThrow();
  });

  it("handles very large values", () => {
    // 3600 seconds = 60 minutes
    expect(formatDuration(3600)).toBe("60m 0s");
    // 7261 seconds = 121 minutes 1 second
    expect(formatDuration(7261)).toBe("121m 1s");
  });

  it("handles fractional seconds", () => {
    expect(formatDuration(30.7)).toBe("30s");
    expect(formatDuration(90.9)).toBe("1m 30s");
  });

  it("handles 1 second", () => {
    expect(formatDuration(1)).toBe("1s");
  });

  it("handles 59 seconds", () => {
    expect(formatDuration(59)).toBe("59s");
  });

  it("handles NaN", () => {
    expect(() => formatDuration(NaN)).not.toThrow();
  });
});
