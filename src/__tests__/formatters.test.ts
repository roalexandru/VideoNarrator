import { describe, it, expect } from "vitest";
import { secondsToTimestamp, formatFileSize, formatDuration } from "../lib/formatters";

describe("secondsToTimestamp — centisecond precision", () => {
  it("handles negative numbers", () => {
    expect(() => secondsToTimestamp(-5)).not.toThrow();
  });

  it("handles zero", () => {
    expect(secondsToTimestamp(0)).toBe("0:00.00");
  });

  it("handles very large values (hours)", () => {
    expect(secondsToTimestamp(3600)).toBe("1:00:00.00");
    expect(secondsToTimestamp(7200)).toBe("2:00:00.00");
  });

  it("handles sub-second fractions", () => {
    expect(secondsToTimestamp(0.1)).toBe("0:00.10");
    expect(secondsToTimestamp(0.99)).toBe("0:00.99");
    expect(secondsToTimestamp(1.5)).toBe("0:01.50");
  });

  it("handles NaN", () => {
    expect(() => secondsToTimestamp(NaN)).not.toThrow();
  });

  it("handles exactly 59 seconds", () => {
    expect(secondsToTimestamp(59)).toBe("0:59.00");
  });

  it("handles exactly 60 seconds", () => {
    expect(secondsToTimestamp(60)).toBe("1:00.00");
  });

  it("handles 61 seconds", () => {
    expect(secondsToTimestamp(61)).toBe("1:01.00");
  });

  it("handles precise timestamps", () => {
    expect(secondsToTimestamp(72.45)).toBe("1:12.45");
    expect(secondsToTimestamp(328.5)).toBe("5:28.50");
    expect(secondsToTimestamp(90.33)).toBe("1:30.33");
  });
});

describe("formatFileSize — edge cases", () => {
  it("handles zero bytes", () => {
    expect(formatFileSize(0)).toBe("0 B");
  });

  it("handles negative bytes", () => {
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

describe("formatDuration — with decimal precision", () => {
  it("handles zero", () => {
    expect(formatDuration(0)).toBe("0.0s");
  });

  it("handles exactly 60 seconds", () => {
    expect(formatDuration(60)).toBe("1m 0.0s");
  });

  it("handles negative numbers", () => {
    expect(() => formatDuration(-10)).not.toThrow();
  });

  it("handles very large values", () => {
    expect(formatDuration(3600)).toBe("60m 0.0s");
    expect(formatDuration(7261)).toBe("121m 1.0s");
  });

  it("handles fractional seconds", () => {
    expect(formatDuration(30.7)).toBe("30.7s");
    expect(formatDuration(90.9)).toBe("1m 30.9s");
  });

  it("handles 1 second", () => {
    expect(formatDuration(1)).toBe("1.0s");
  });

  it("handles 59 seconds", () => {
    expect(formatDuration(59)).toBe("59.0s");
  });

  it("handles NaN", () => {
    expect(() => formatDuration(NaN)).not.toThrow();
  });
});
