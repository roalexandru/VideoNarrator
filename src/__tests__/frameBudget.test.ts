import { describe, it, expect } from "vitest";
import { recommendedMaxFrames, isReasoningModel, DENSITY_INTERVAL } from "../lib/frameBudget";

describe("recommendedMaxFrames", () => {
  it("uses the 30-frame floor for short videos", () => {
    // 60s @ heavy (2s interval) = 30 frames — at the floor.
    expect(recommendedMaxFrames(60, "heavy")).toBe(30);
    // 10s @ heavy = 5 raw, floored to 30.
    expect(recommendedMaxFrames(10, "heavy")).toBe(30);
    // 100s @ light (10s) = 10 raw, floored to 30.
    expect(recommendedMaxFrames(100, "light")).toBe(30);
  });

  it("scales up with duration above the floor", () => {
    // 300s @ heavy (2s) = 150 frames.
    expect(recommendedMaxFrames(300, "heavy")).toBe(150);
    // 600s @ medium (5s) = 120 frames.
    expect(recommendedMaxFrames(600, "medium")).toBe(120);
    // 600s @ light (10s) = 60 frames.
    expect(recommendedMaxFrames(600, "light")).toBe(60);
  });

  it("caps at 300 to prevent pathological requests", () => {
    // 3600s (1hr) @ heavy would be 1800 — capped at 300.
    expect(recommendedMaxFrames(3600, "heavy")).toBe(300);
    // 10000s @ medium would be 2000 — capped.
    expect(recommendedMaxFrames(10000, "medium")).toBe(300);
  });

  it("returns 30 for zero or negative durations", () => {
    expect(recommendedMaxFrames(0, "medium")).toBe(30);
    expect(recommendedMaxFrames(-5, "heavy")).toBe(30);
  });

  it("rounds up so partial intervals still get a frame", () => {
    // 61s @ heavy (2s) = 30.5 → ceil → 31. But floor is 30, so 31 wins.
    expect(recommendedMaxFrames(61, "heavy")).toBe(31);
  });
});

describe("DENSITY_INTERVAL", () => {
  it("matches the Rust FrameDensity::interval_seconds values", () => {
    // Kept in lockstep with src-tauri/src/models.rs — if this test fails,
    // someone changed one side without the other.
    expect(DENSITY_INTERVAL.light).toBe(10);
    expect(DENSITY_INTERVAL.medium).toBe(5);
    expect(DENSITY_INTERVAL.heavy).toBe(2);
  });
});

describe("isReasoningModel", () => {
  it("returns true for OpenAI reasoning model families", () => {
    expect(isReasoningModel("openai", "o1-preview")).toBe(true);
    expect(isReasoningModel("openai", "o3")).toBe(true);
    expect(isReasoningModel("openai", "o3-mini")).toBe(true);
    expect(isReasoningModel("openai", "o4-mini")).toBe(true);
    expect(isReasoningModel("openai", "gpt-5")).toBe(true);
    expect(isReasoningModel("openai", "gpt-5-turbo")).toBe(true);
  });

  it("returns false for non-reasoning OpenAI models", () => {
    expect(isReasoningModel("openai", "gpt-4o")).toBe(false);
    expect(isReasoningModel("openai", "gpt-4-turbo")).toBe(false);
  });

  it("returns false for non-OpenAI providers even with matching model names", () => {
    // The backend gate is openai-specific; a provider named "claude" with a
    // hypothetical "o1" model must not be treated as reasoning.
    expect(isReasoningModel("claude", "o1")).toBe(false);
    expect(isReasoningModel("gemini", "gpt-5")).toBe(false);
  });
});
