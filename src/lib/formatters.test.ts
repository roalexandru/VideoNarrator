import { describe, it, expect } from "vitest";
import { secondsToTimestamp, formatFileSize, formatDuration } from "./formatters";

describe("secondsToTimestamp", () => {
  it("formats zero", () => {
    expect(secondsToTimestamp(0)).toBe("0:00");
  });
  it("formats seconds only", () => {
    expect(secondsToTimestamp(45)).toBe("0:45");
  });
  it("formats minutes and seconds", () => {
    expect(secondsToTimestamp(125)).toBe("2:05");
  });
  it("handles fractional seconds", () => {
    expect(secondsToTimestamp(65.7)).toBe("1:05");
  });
});

describe("formatFileSize", () => {
  it("formats bytes", () => {
    expect(formatFileSize(500)).toBe("500 B");
  });
  it("formats kilobytes", () => {
    expect(formatFileSize(1500)).toBe("1.5 KB");
  });
  it("formats megabytes", () => {
    expect(formatFileSize(5 * 1024 * 1024)).toBe("5.0 MB");
  });
  it("formats gigabytes", () => {
    expect(formatFileSize(2 * 1024 * 1024 * 1024)).toBe("2.0 GB");
  });
});

describe("formatDuration", () => {
  it("formats seconds only", () => {
    expect(formatDuration(30)).toBe("30s");
  });
  it("formats minutes and seconds", () => {
    expect(formatDuration(125)).toBe("2m 5s");
  });
  it("formats zero", () => {
    expect(formatDuration(0)).toBe("0s");
  });
});
