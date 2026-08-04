import { describe, it, expect, beforeEach } from "vitest";
import { useConfigStore } from "./configStore";
import {
  PROVIDERS,
  DEFAULT_MODEL_FOR_PROVIDER,
  REASONING_LEVELS,
} from "../lib/constants";

describe("configStore", () => {
  beforeEach(() => {
    useConfigStore.getState().reset();
  });

  it("has correct initial state", () => {
    const state = useConfigStore.getState();
    expect(state.style).toBe("product_demo");
    expect(state.languages).toEqual(["en"]);
    expect(state.primaryLanguage).toBe("en");
    expect(state.frameDensity).toBe("medium");
    expect(state.sceneThreshold).toBe(0.3);
    expect(state.maxFrames).toBe(30);
    expect(state.customPrompt).toBe("");
    expect(state.aiProvider).toBe("claude");
    expect(state.model).toBe("claude-sonnet-5");
    expect(state.temperature).toBe(0.7);
    expect(state.reasoningEffort).toBe("balanced");
    expect(state.ttsProvider).toBe("elevenlabs");
    expect(state.strictMode).toBe(false);
    // Grounding features ship enabled — a regression here silently reverts
    // narration to the pre-0.10 behaviour without any visible signal.
    expect(state.screenText).toBe(true);
    expect(state.modelFrameSelection).toBe(true);
  });

  it("sets style", () => {
    useConfigStore.getState().setStyle("technical");
    expect(useConfigStore.getState().style).toBe("technical");
  });

  it("toggles language on and off", () => {
    useConfigStore.getState().toggleLanguage("ja");
    expect(useConfigStore.getState().languages).toEqual(["en", "ja"]);

    useConfigStore.getState().toggleLanguage("ja");
    expect(useConfigStore.getState().languages).toEqual(["en"]);
  });

  it("falls back primary language when removed", () => {
    useConfigStore.getState().toggleLanguage("ja");
    useConfigStore.getState().setPrimaryLanguage("ja");
    expect(useConfigStore.getState().primaryLanguage).toBe("ja");

    // Remove ja — primary should fall back to first available
    useConfigStore.getState().toggleLanguage("ja");
    expect(useConfigStore.getState().primaryLanguage).toBe("en");
  });

  it("sets temperature", () => {
    useConfigStore.getState().setTemperature(0.3);
    expect(useConfigStore.getState().temperature).toBe(0.3);
  });

  it("sets AI provider and model", () => {
    useConfigStore.getState().setAiProvider("openai");
    useConfigStore.getState().setModel("gpt-4o");
    expect(useConfigStore.getState().aiProvider).toBe("openai");
    expect(useConfigStore.getState().model).toBe("gpt-4o");
  });

  it("sets TTS provider", () => {
    useConfigStore.getState().setTtsProvider("azure");
    expect(useConfigStore.getState().ttsProvider).toBe("azure");
  });

  it("toggles strict mode", () => {
    expect(useConfigStore.getState().strictMode).toBe(false);
    useConfigStore.getState().setStrictMode(true);
    expect(useConfigStore.getState().strictMode).toBe(true);
    useConfigStore.getState().setStrictMode(false);
    expect(useConfigStore.getState().strictMode).toBe(false);
  });

  it("toggles on-screen text reading", () => {
    expect(useConfigStore.getState().screenText).toBe(true);
    useConfigStore.getState().setScreenText(false);
    expect(useConfigStore.getState().screenText).toBe(false);
    useConfigStore.getState().setScreenText(true);
    expect(useConfigStore.getState().screenText).toBe(true);
  });

  it("toggles model-driven frame selection", () => {
    expect(useConfigStore.getState().modelFrameSelection).toBe(true);
    useConfigStore.getState().setModelFrameSelection(false);
    expect(useConfigStore.getState().modelFrameSelection).toBe(false);
    useConfigStore.getState().setModelFrameSelection(true);
    expect(useConfigStore.getState().modelFrameSelection).toBe(true);
  });

  it("resets to initial state", () => {
    useConfigStore.getState().setStyle("technical");
    useConfigStore.getState().setTemperature(0.2);
    useConfigStore.getState().setAiProvider("openai");
    useConfigStore.getState().setStrictMode(true);
    useConfigStore.getState().setScreenText(false);
    useConfigStore.getState().setModelFrameSelection(false);
    useConfigStore.getState().reset();

    const state = useConfigStore.getState();
    expect(state.style).toBe("product_demo");
    expect(state.temperature).toBe(0.7);
    expect(state.aiProvider).toBe("claude");
    expect(state.strictMode).toBe(false);
    // Reset must restore the enabled defaults, not fall back to `false`.
    expect(state.screenText).toBe(true);
    expect(state.modelFrameSelection).toBe(true);
  });
});

describe("model catalog", () => {
  it("offers only current-generation models, with a default per provider", () => {
    for (const p of PROVIDERS) {
      expect(p.models.length).toBeGreaterThan(0);
      // No retired/legacy IDs in the picker — those stay type-valid only so
      // older saved projects still load.
      for (const m of p.models) {
        expect(m.id).not.toMatch(/-2025\d{4}$|^gpt-4o$|^o3$|^gemini-2\.5/);
      }
      // The per-provider default must actually be offered by that provider.
      const dflt = DEFAULT_MODEL_FOR_PROVIDER[p.id];
      expect(p.models.map((m) => m.id)).toContain(dflt);
    }
  });

  it("includes the GPT-5.6 Sol/Terra/Luna variants", () => {
    const openai = PROVIDERS.find((p) => p.id === "openai")!;
    const ids = openai.models.map((m) => m.id);
    expect(ids).toEqual(
      expect.arrayContaining(["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"]),
    );
  });

  it("includes Claude Opus 5 and Sonnet 5", () => {
    const claude = PROVIDERS.find((p) => p.id === "claude")!;
    const ids = claude.models.map((m) => m.id);
    expect(ids).toEqual(expect.arrayContaining(["claude-opus-5", "claude-sonnet-5"]));
  });

  it("exposes four reasoning levels including the store default", () => {
    expect(REASONING_LEVELS.map((l) => l.id)).toEqual([
      "fast",
      "balanced",
      "thorough",
      "max",
    ]);
    const ids = REASONING_LEVELS.map((l) => l.id);
    expect(ids).toContain(useConfigStore.getState().reasoningEffort);
  });
});
