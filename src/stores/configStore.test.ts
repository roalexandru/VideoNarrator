import { describe, it, expect, beforeEach } from "vitest";
import { useConfigStore } from "./configStore";

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
    expect(state.model).toBe("claude-sonnet-4-20250514");
    expect(state.temperature).toBe(0.7);
    expect(state.ttsProvider).toBe("elevenlabs");
    expect(state.strictMode).toBe(false);
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

  it("resets to initial state", () => {
    useConfigStore.getState().setStyle("technical");
    useConfigStore.getState().setTemperature(0.2);
    useConfigStore.getState().setAiProvider("openai");
    useConfigStore.getState().setStrictMode(true);
    useConfigStore.getState().reset();

    const state = useConfigStore.getState();
    expect(state.style).toBe("product_demo");
    expect(state.temperature).toBe(0.7);
    expect(state.aiProvider).toBe("claude");
    expect(state.strictMode).toBe(false);
  });
});
