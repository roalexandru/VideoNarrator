import { describe, it, expect, beforeEach } from "vitest";
import { useExportStore, slugify } from "./exportStore";

describe("slugify", () => {
  it("converts spaces to hyphens", () => {
    expect(slugify("My Cool Project")).toBe("my-cool-project");
  });

  it("removes special characters", () => {
    expect(slugify("Project @#$% Demo!")).toBe("project-demo");
  });

  it("collapses multiple hyphens", () => {
    expect(slugify("a   b---c")).toBe("a-b-c");
  });

  it("returns 'untitled' for empty string", () => {
    expect(slugify("")).toBe("untitled");
  });

  it("returns 'untitled' for only special characters", () => {
    expect(slugify("@#$%")).toBe("untitled");
  });
});

describe("exportStore", () => {
  beforeEach(() => {
    useExportStore.getState().reset();
  });

  it("has correct initial state", () => {
    const state = useExportStore.getState();
    expect(state.selectedFormats).toEqual(["json", "srt"]);
    expect(state.languageToggles).toEqual({ en: true });
    expect(state.outputDirectory).toBeNull();
    expect(state.basename).toBe("untitled");
    expect(state.burnSubtitles).toBe(false);
    expect(state.replaceAudio).toBe(true);
  });

  it("toggles format on and off", () => {
    useExportStore.getState().toggleFormat("vtt");
    expect(useExportStore.getState().selectedFormats).toContain("vtt");

    useExportStore.getState().toggleFormat("vtt");
    expect(useExportStore.getState().selectedFormats).not.toContain("vtt");
  });

  it("toggles language export", () => {
    useExportStore.getState().toggleLanguageExport("en");
    expect(useExportStore.getState().languageToggles.en).toBe(false);

    useExportStore.getState().toggleLanguageExport("en");
    expect(useExportStore.getState().languageToggles.en).toBe(true);
  });

  it("initializes languages", () => {
    useExportStore.getState().initLanguages(["en", "ja", "de"]);
    const toggles = useExportStore.getState().languageToggles;
    expect(toggles).toEqual({ en: true, ja: true, de: true });
  });

  it("initializes from title", () => {
    useExportStore.getState().initFromTitle("My Demo Video", "/Users/test");
    const state = useExportStore.getState();
    expect(state.basename).toBe("my-demo-video");
    expect(state.outputDirectory).toBe("/Users/test/Documents/Narrator/my-demo-video");
  });

  it("does not overwrite existing basename if output dir is set", () => {
    useExportStore.getState().setBasename("custom-name");
    useExportStore.getState().setOutputDirectory("/custom/path");
    useExportStore.getState().initFromTitle("New Title", "/Users/test");
    expect(useExportStore.getState().basename).toBe("custom-name");
  });

  it("resets to initial state", () => {
    useExportStore.getState().toggleFormat("vtt");
    useExportStore.getState().setBasename("custom");
    useExportStore.getState().reset();

    const state = useExportStore.getState();
    expect(state.selectedFormats).toEqual(["json", "srt"]);
    expect(state.basename).toBe("untitled");
  });
});
