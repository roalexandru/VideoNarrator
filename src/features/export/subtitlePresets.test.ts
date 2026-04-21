import { describe, it, expect } from "vitest";
import { detectPreset, SUBTITLE_PRESETS, type SubtitleStyleFields } from "./subtitlePresets";

describe("detectPreset", () => {
  it("identifies an exact shorts match", () => {
    const fields: SubtitleStyleFields = { ...SUBTITLE_PRESETS.shorts };
    expect(detectPreset(fields)).toBe("shorts");
  });

  it("identifies an exact documentary match", () => {
    const fields: SubtitleStyleFields = { ...SUBTITLE_PRESETS.documentary };
    expect(detectPreset(fields)).toBe("documentary");
  });

  it("identifies an exact clean match", () => {
    const fields: SubtitleStyleFields = { ...SUBTITLE_PRESETS.clean };
    expect(detectPreset(fields)).toBe("clean");
  });

  it("returns custom when a single field diverges", () => {
    const fields: SubtitleStyleFields = {
      ...SUBTITLE_PRESETS.documentary,
      subtitleFontSize: 25, // one tick off
    };
    expect(detectPreset(fields)).toBe("custom");
  });

  it("returns custom when color diverges", () => {
    const fields: SubtitleStyleFields = {
      ...SUBTITLE_PRESETS.clean,
      subtitleColor: "#ffff00",
    };
    expect(detectPreset(fields)).toBe("custom");
  });

  it("does not collapse two presets sharing a subset of fields", () => {
    // Documentary and clean share color/outline_color/position; only font size
    // and outline width differ. Confirm the detector doesn't report the wrong
    // one when one field is ambiguous.
    const fields: SubtitleStyleFields = {
      ...SUBTITLE_PRESETS.documentary,
      subtitleOutline: 1, // steals from clean
    };
    expect(detectPreset(fields)).toBe("custom");
  });
});
