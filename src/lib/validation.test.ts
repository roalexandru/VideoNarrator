import { describe, it, expect } from "vitest";
import {
  canProceedFromStep0,
  canProceedFromStep1,
  canProceedFromStep3,
} from "./validation";

describe("canProceedFromStep0", () => {
  it("returns false when no video", () => {
    expect(canProceedFromStep0(null, "title")).toBe(false);
  });

  it("returns false when no title", () => {
    const video = {
      path: "/test.mp4",
      name: "test.mp4",
      size: 1000,
      duration: 30,
      resolution: { width: 1920, height: 1080 },
      codec: "h264",
      fps: 30,
    };
    expect(canProceedFromStep0(video, "")).toBe(false);
    expect(canProceedFromStep0(video, "  ")).toBe(false);
  });

  it("returns true when video and title present", () => {
    const video = {
      path: "/test.mp4",
      name: "test.mp4",
      size: 1000,
      duration: 30,
      resolution: { width: 1920, height: 1080 },
      codec: "h264",
      fps: 30,
    };
    expect(canProceedFromStep0(video, "My Video")).toBe(true);
  });
});

describe("canProceedFromStep1", () => {
  it("returns false with no languages", () => {
    expect(canProceedFromStep1([])).toBe(false);
  });
  it("returns true with languages", () => {
    expect(canProceedFromStep1(["en"])).toBe(true);
  });
});

describe("canProceedFromStep3", () => {
  it("returns false with no segments", () => {
    expect(canProceedFromStep3(0)).toBe(false);
  });
  it("returns true with segments", () => {
    expect(canProceedFromStep3(3)).toBe(true);
  });
});
