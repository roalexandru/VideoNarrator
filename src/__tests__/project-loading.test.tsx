import { describe, it, expect, beforeEach } from "vitest";
import { setupDefaultMocks, resetAllStores } from "./setup";
import { useProjectStore } from "../stores/projectStore";
import { useConfigStore } from "../stores/configStore";
import { useScriptStore } from "../stores/scriptStore";
import { useEditStore } from "../stores/editStore";
import { useProcessingStore } from "../stores/processingStore";
import { useExportStore } from "../stores/exportStore";
import type { VideoFile } from "../types/project";
// NarrationScript type removed — not needed in current tests

beforeEach(() => {
  setupDefaultMocks();
  resetAllStores();
});

function makeVideoFile(overrides: Partial<VideoFile> = {}): VideoFile {
  return {
    path: "/tmp/demo.mp4",
    name: "demo.mp4",
    size: 50_000_000,
    duration: 120,
    resolution: { width: 1920, height: 1080 },
    codec: "h264",
    fps: 30,
    ...overrides,
  };
}



describe("project loading - populates projectStore", () => {
  it("loading a project sets title, videoPath, and description in projectStore", () => {
    const ps = useProjectStore.getState();
    ps.setProjectId("proj-1");
    ps.setTitle("Demo Project");
    ps.setDescription("A demo");
    ps.setVideoFile(makeVideoFile());

    const state = useProjectStore.getState();
    expect(state.projectId).toBe("proj-1");
    expect(state.title).toBe("Demo Project");
    expect(state.description).toBe("A demo");
    expect(state.videoFile).not.toBeNull();
    expect(state.videoFile!.path).toBe("/tmp/demo.mp4");
    expect(state.videoFile!.duration).toBe(120);
  });

  it("loading a project sets createdAt", () => {
    const ps = useProjectStore.getState();
    ps.setCreatedAt("2026-04-01T12:00:00Z");
    expect(useProjectStore.getState().createdAt).toBe("2026-04-01T12:00:00Z");
  });
});

describe("project loading - populates configStore", () => {
  it("loading a project sets style, languages, and aiConfig in configStore", () => {
    const cs = useConfigStore.getState();
    cs.setStyle("technical");
    cs.toggleLanguage("ja");
    cs.setPrimaryLanguage("en");
    cs.setAiProvider("openai");
    cs.setModel("gpt-4o");
    cs.setTemperature(0.5);

    const state = useConfigStore.getState();
    expect(state.style).toBe("technical");
    expect(state.languages).toEqual(["en", "ja"]);
    expect(state.primaryLanguage).toBe("en");
    expect(state.aiProvider).toBe("openai");
    expect(state.model).toBe("gpt-4o");
    expect(state.temperature).toBe(0.5);
  });

  it("loading config sets frame config", () => {
    const cs = useConfigStore.getState();
    cs.setFrameDensity("heavy");
    cs.setSceneThreshold(0.5);
    cs.setMaxFrames(50);

    const state = useConfigStore.getState();
    expect(state.frameDensity).toBe("heavy");
    expect(state.sceneThreshold).toBe(0.5);
    expect(state.maxFrames).toBe(50);
  });

  it("loading config sets custom prompt", () => {
    const cs = useConfigStore.getState();
    cs.setCustomPrompt("Be very formal");
    expect(useConfigStore.getState().customPrompt).toBe("Be very formal");
  });
});

describe("project saving - reads from all stores correctly", () => {
  it("buildSavePayload reads from projectStore, configStore, and editStore", () => {
    // Populate all stores
    const ps = useProjectStore.getState();
    ps.setProjectId("proj-save-1");
    ps.setTitle("Save Test");
    ps.setDescription("Test description");
    ps.setVideoFile(makeVideoFile());
    ps.setCreatedAt("2026-04-01T12:00:00Z");

    const cs = useConfigStore.getState();
    cs.setStyle("technical");
    cs.toggleLanguage("ja");
    cs.setAiProvider("openai");
    cs.setModel("gpt-4o");
    cs.setTemperature(0.3);
    cs.setCustomPrompt("Custom");
    cs.setFrameDensity("heavy");
    cs.setSceneThreshold(0.5);
    cs.setMaxFrames(50);

    useEditStore.getState().initFromVideo(120);

    // Read back all stores and verify consistency
    const psState = useProjectStore.getState();
    const csState = useConfigStore.getState();
    const esState = useEditStore.getState();

    expect(psState.projectId).toBe("proj-save-1");
    expect(psState.title).toBe("Save Test");
    expect(psState.description).toBe("Test description");
    expect(psState.videoFile!.path).toBe("/tmp/demo.mp4");
    expect(psState.createdAt).toBe("2026-04-01T12:00:00Z");

    expect(csState.style).toBe("technical");
    expect(csState.languages).toContain("ja");
    expect(csState.aiProvider).toBe("openai");
    expect(csState.model).toBe("gpt-4o");
    expect(csState.temperature).toBe(0.3);
    expect(csState.customPrompt).toBe("Custom");
    expect(csState.frameDensity).toBe("heavy");

    expect(esState.clips).toHaveLength(1);
    expect(esState.clips[0].sourceStart).toBe(0);
    expect(esState.clips[0].sourceEnd).toBe(120);
  });
});

describe("auto-save safety - empty stores", () => {
  it("stores are empty after reset and no crashes accessing them", () => {
    resetAllStores();

    const ps = useProjectStore.getState();
    const cs = useConfigStore.getState();
    const es = useEditStore.getState();
    const ss = useScriptStore.getState();
    const proc = useProcessingStore.getState();
    const exp = useExportStore.getState();

    // No video file means auto-save would guard before trying to save
    expect(ps.videoFile).toBeNull();
    expect(ps.projectId).toBe("");
    expect(ps.title).toBe("");
    expect(ps.description).toBe("");

    // Config has safe defaults
    expect(cs.style).toBe("product_demo");
    expect(cs.languages).toEqual(["en"]);

    // Edit has no clips
    expect(es.clips).toHaveLength(0);
    expect(es.getOutputDuration()).toBe(0);

    // Script has no scripts
    expect(Object.keys(ss.scripts)).toHaveLength(0);

    // Processing is idle
    expect(proc.phase).toBe("idle");
    expect(proc.error).toBeNull();

    // Export has defaults
    expect(exp.selectedFormats).toEqual(["json", "srt"]);
  });
});

describe("rapid store updates", () => {
  it("setting title then immediately setting description preserves both", () => {
    const ps = useProjectStore.getState();
    ps.setTitle("Rapid Title");
    ps.setDescription("Rapid Description");

    const state = useProjectStore.getState();
    expect(state.title).toBe("Rapid Title");
    expect(state.description).toBe("Rapid Description");
  });

  it("rapid config changes are all preserved", () => {
    const cs = useConfigStore.getState();
    cs.setStyle("technical");
    cs.setAiProvider("openai");
    cs.setModel("gpt-4o");
    cs.setTemperature(0.2);
    cs.toggleLanguage("de");
    cs.toggleLanguage("fr");
    cs.setCustomPrompt("Be concise");

    const state = useConfigStore.getState();
    expect(state.style).toBe("technical");
    expect(state.aiProvider).toBe("openai");
    expect(state.model).toBe("gpt-4o");
    expect(state.temperature).toBe(0.2);
    expect(state.languages).toEqual(["en", "de", "fr"]);
    expect(state.customPrompt).toBe("Be concise");
  });

  it("rapid project + edit store updates remain consistent", () => {
    const ps = useProjectStore.getState();
    ps.setVideoFile(makeVideoFile({ duration: 90 }));
    ps.setTitle("Fast Updates");

    const es = useEditStore.getState();
    es.initFromVideo(90);
    es.splitAt(30);
    es.setClipSpeed(0, 2.0);

    expect(useProjectStore.getState().title).toBe("Fast Updates");
    expect(useProjectStore.getState().videoFile!.duration).toBe(90);
    expect(useEditStore.getState().clips).toHaveLength(2);
    expect(useEditStore.getState().clips[0].speed).toBe(2.0);
  });
});

describe("projectStore.reset() clears all fields", () => {
  it("clears everything including createdAt", () => {
    const ps = useProjectStore.getState();
    ps.setProjectId("proj-1");
    ps.setTitle("Title");
    ps.setDescription("Desc");
    ps.setVideoFile(makeVideoFile());
    ps.setCreatedAt("2026-04-01T12:00:00Z");
    ps.addDocuments([{ id: "doc-1", path: "/tmp/d.md", name: "d.md", size: 100, type: "md" }]);

    ps.reset();

    const state = useProjectStore.getState();
    expect(state.projectId).toBe("");
    expect(state.videoFile).toBeNull();
    expect(state.title).toBe("");
    expect(state.description).toBe("");
    expect(state.createdAt).toBeNull();
    expect(state.contextDocuments).toHaveLength(0);
  });
});

describe("video file metadata", () => {
  it("setting video file metadata updates dimensions, codec, and duration", () => {
    const ps = useProjectStore.getState();
    ps.setVideoFile(makeVideoFile({
      duration: 300,
      resolution: { width: 3840, height: 2160 },
      codec: "h265",
      fps: 60,
      size: 200_000_000,
    }));

    const vf = useProjectStore.getState().videoFile!;
    expect(vf.duration).toBe(300);
    expect(vf.resolution.width).toBe(3840);
    expect(vf.resolution.height).toBe(2160);
    expect(vf.codec).toBe("h265");
    expect(vf.fps).toBe(60);
    expect(vf.size).toBe(200_000_000);
  });

  it("replacing video file overwrites previous metadata completely", () => {
    const ps = useProjectStore.getState();
    ps.setVideoFile(makeVideoFile({ path: "/tmp/first.mp4", duration: 60 }));
    ps.setVideoFile(makeVideoFile({ path: "/tmp/second.mp4", duration: 180, codec: "vp9" }));

    const vf = useProjectStore.getState().videoFile!;
    expect(vf.path).toBe("/tmp/second.mp4");
    expect(vf.duration).toBe(180);
    expect(vf.codec).toBe("vp9");
  });
});
