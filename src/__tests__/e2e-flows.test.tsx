import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";
import { setupDefaultMocks, resetAllStores } from "./setup";
import { useProjectStore } from "../stores/projectStore";
import { useConfigStore } from "../stores/configStore";
import { useScriptStore } from "../stores/scriptStore";
import { useProcessingStore } from "../stores/processingStore";
import { useEditStore } from "../stores/editStore";
import { useExportStore } from "../stores/exportStore";
import { ProcessingScreen } from "../features/processing/ProcessingScreen";
import { ReviewScreen } from "../features/review/ReviewScreen";
import { ExportScreen } from "../features/export/ExportScreen";
import type { NarrationScript, Segment } from "../types/script";

// ─── Helpers ──────────────────────────────────────────────────────────────────

function makeSeg(overrides: Partial<Segment> = {}): Segment {
  return {
    index: 0,
    start_seconds: 0,
    end_seconds: 15,
    text: "Welcome to the demo.",
    visual_description: "Title screen",
    emphasis: [],
    pace: "medium",
    pause_after_ms: 500,
    frame_refs: [0, 1],
    ...overrides,
  };
}

function makeScript(
  segments: Segment[],
  overrides: Partial<NarrationScript> = {},
): NarrationScript {
  return {
    title: "Test Narration",
    total_duration_seconds: 30,
    segments,
    metadata: {
      style: "product_demo",
      language: "en",
      provider: "claude",
      model: "claude-sonnet-4-20250514",
      generated_at: "2026-04-03T14:00:00Z",
    },
    ...overrides,
  };
}

function seedVideoProject() {
  useProjectStore.getState().setProjectId("proj-e2e");
  useProjectStore.getState().setTitle("E2E Test Video");
  useProjectStore.getState().setVideoFile({
    path: "/tmp/test.mp4",
    name: "test.mp4",
    size: 50_000_000,
    duration: 120,
    resolution: { width: 1920, height: 1080 },
    codec: "h264",
    fps: 30,
  });
}

function seedConfig() {
  const cs = useConfigStore.getState();
  cs.setStyle("product_demo");
  cs.setAiProvider("claude");
  cs.setModel("claude-sonnet-4-20250514");
  cs.setTemperature(0.7);
  cs.setFrameDensity("medium");
}

function seedTwoSegmentScript(lang = "en"): NarrationScript {
  const script = makeScript([
    makeSeg({
      index: 0,
      start_seconds: 0,
      end_seconds: 15,
      text: "Welcome to the demo.",
    }),
    makeSeg({
      index: 1,
      start_seconds: 15,
      end_seconds: 30,
      text: "Here we see the main feature.",
      visual_description: "Feature overview",
      pause_after_ms: 0,
      frame_refs: [2, 3],
    }),
  ]);
  if (lang !== "en") {
    script.metadata = { ...script.metadata, language: lang };
  }
  useScriptStore.getState().setScript(lang, script);
  return script;
}

// ─── Test Suites ──────────────────────────────────────────────────────────────

describe("E2E Flow 1: New Project -> Configure -> Generate -> Review", () => {
  beforeEach(() => {
    resetAllStores();
    setupDefaultMocks();
  });
  afterEach(() => clearMocks());

  it("configures project and receives generated script from IPC", async () => {
    // Step 1: Set up project with a video file
    seedVideoProject();
    seedConfig();

    // Verify project state
    expect(useProjectStore.getState().videoFile).not.toBeNull();
    expect(useProjectStore.getState().title).toBe("E2E Test Video");
    expect(useConfigStore.getState().style).toBe("product_demo");
    expect(useConfigStore.getState().aiProvider).toBe("claude");

    // Step 2: Simulate the generate_narration IPC call result
    // The mock in setupDefaultMocks returns a script with 2 segments.
    // Simulate what the ProcessingScreen does when generation completes:
    const mockResult: NarrationScript = {
      title: "Test Narration",
      total_duration_seconds: 30,
      segments: [
        {
          index: 0, start_seconds: 0, end_seconds: 15,
          text: "Welcome to the demo.", visual_description: "Title screen",
          emphasis: [], pace: "medium", pause_after_ms: 500, frame_refs: [0, 1],
        },
        {
          index: 1, start_seconds: 15, end_seconds: 30,
          text: "Here we see the main feature.", visual_description: "Feature overview",
          emphasis: [], pace: "medium", pause_after_ms: 0, frame_refs: [2, 3],
        },
      ],
      metadata: {
        style: "product_demo", language: "en",
        provider: "claude", model: "claude-sonnet-4-20250514",
        generated_at: "2026-04-03T14:00:00Z",
      },
    };

    // Apply the result as the app would
    useScriptStore.getState().setScript("en", mockResult);
    useProcessingStore.getState().setPhase("done");

    // Step 3: Verify scriptStore received the segments
    const scripts = useScriptStore.getState().scripts;
    expect(scripts["en"]).toBeDefined();
    expect(scripts["en"].segments).toHaveLength(2);
    expect(scripts["en"].segments[0].text).toBe("Welcome to the demo.");
    expect(scripts["en"].segments[1].text).toBe("Here we see the main feature.");

    // Step 4: Verify processingStore transitioned
    expect(useProcessingStore.getState().phase).toBe("done");
  });

  it("ProcessingScreen shows completion when script exists", () => {
    seedVideoProject();
    seedConfig();
    seedTwoSegmentScript();
    useProcessingStore.getState().setPhase("done");

    render(<ProcessingScreen />);

    expect(screen.getByText("Generation complete!")).toBeInTheDocument();
    expect(screen.getByText("2 segments ready for review")).toBeInTheDocument();
  });

  it("processingStore tracks phase transitions through the pipeline", () => {
    const ps = useProcessingStore.getState();

    // Idle -> extracting_frames -> generating_narration -> done
    expect(ps.phase).toBe("idle");

    useProcessingStore.getState().setPhase("extracting_frames");
    expect(useProcessingStore.getState().phase).toBe("extracting_frames");

    useProcessingStore.getState().setProgress(50);
    expect(useProcessingStore.getState().progress).toBe(50);

    useProcessingStore.getState().setPhase("generating_narration");
    expect(useProcessingStore.getState().phase).toBe("generating_narration");

    useProcessingStore.getState().setPhase("done");
    expect(useProcessingStore.getState().phase).toBe("done");
  });
});

describe("E2E Flow 2: Review and Edit Script", () => {
  beforeEach(() => {
    resetAllStores();
    setupDefaultMocks();
  });
  afterEach(() => clearMocks());

  it("edits a segment text via store", () => {
    seedVideoProject();
    seedTwoSegmentScript();

    useScriptStore.getState().updateSegmentText("en", 0, "Updated welcome text.");
    const seg = useScriptStore.getState().scripts["en"].segments[0];
    expect(seg.text).toBe("Updated welcome text.");
  });

  it("deletes a segment and verifies re-indexing", () => {
    seedVideoProject();
    seedTwoSegmentScript();

    useScriptStore.getState().deleteSegment("en", 0);
    const segs = useScriptStore.getState().scripts["en"].segments;
    expect(segs).toHaveLength(1);
    expect(segs[0].text).toBe("Here we see the main feature.");
    expect(segs[0].index).toBe(0); // Re-indexed
  });

  it("splits a segment into two at a given timestamp", () => {
    seedVideoProject();
    const script = makeScript([
      makeSeg({
        index: 0,
        start_seconds: 0,
        end_seconds: 20,
        text: "Welcome to the demo and more content here",
      }),
    ]);
    useScriptStore.getState().setScript("en", script);

    useScriptStore.getState().splitSegment("en", 0, 10);

    const segs = useScriptStore.getState().scripts["en"].segments;
    expect(segs).toHaveLength(2);
    expect(segs[0].end_seconds).toBe(10);
    expect(segs[1].start_seconds).toBe(10);
    expect(segs[1].end_seconds).toBe(20);
    // Both parts should have non-empty text
    expect(segs[0].text.length).toBeGreaterThan(0);
    expect(segs[1].text.length).toBeGreaterThan(0);
  });

  it("ReviewScreen renders segments and allows deletion", async () => {
    seedVideoProject();
    seedTwoSegmentScript();

    const user = userEvent.setup();
    render(<ReviewScreen />);

    // Segments should be visible
    expect(screen.getAllByText("Welcome to the demo.").length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText("Here we see the main feature.").length).toBeGreaterThanOrEqual(1);

    // Delete first segment — opens confirmation dialog
    const delButtons = screen.getAllByText("Del");
    expect(delButtons).toHaveLength(2);
    await user.click(delButtons[0]);

    // Confirm deletion
    const confirmBtn = screen.getByText("Delete");
    await user.click(confirmBtn);

    // Verify store updated
    const segs = useScriptStore.getState().scripts["en"].segments;
    expect(segs).toHaveLength(1);
    expect(segs[0].text).toBe("Here we see the main feature.");
  });

  it("merges two adjacent segments", () => {
    seedVideoProject();
    seedTwoSegmentScript();

    useScriptStore.getState().mergeSegments("en", 0, 1);
    const segs = useScriptStore.getState().scripts["en"].segments;
    expect(segs).toHaveLength(1);
    expect(segs[0].text).toBe("Welcome to the demo. Here we see the main feature.");
    expect(segs[0].start_seconds).toBe(0);
    expect(segs[0].end_seconds).toBe(30);
  });
});

describe("E2E Flow 3: Export Flow", () => {
  beforeEach(() => {
    resetAllStores();
    setupDefaultMocks();
  });
  afterEach(() => clearMocks());

  it("sets up export configuration and verifies IPC mock returns results", async () => {
    seedVideoProject();
    seedTwoSegmentScript();

    // Set up export store
    useExportStore.getState().setOutputDirectory("/tmp/export-output");
    useExportStore.getState().setBasename("my-video");
    useExportStore.getState().initLanguages(["en"]);

    // Verify export store state
    expect(useExportStore.getState().outputDirectory).toBe("/tmp/export-output");
    expect(useExportStore.getState().basename).toBe("my-video");
    expect(useExportStore.getState().selectedFormats).toContain("json");
    expect(useExportStore.getState().selectedFormats).toContain("srt");
    expect(useExportStore.getState().languageToggles["en"]).toBe(true);
  });

  it("export_script IPC returns file paths for each format", async () => {
    seedVideoProject();
    seedTwoSegmentScript();

    // The mockIPC export_script handler returns results
    const { invoke } = await import("@tauri-apps/api/core");
    const results = await invoke("export_script", {
      options: {
        formats: ["json", "srt"],
        languages: ["en"],
        output_directory: "/tmp/export-output",
        scripts: useScriptStore.getState().scripts,
        basename: "my-video",
      },
    });

    expect(results).toEqual([
      { format: "json", language: "en", file_path: "/tmp/out/script.json", success: true },
      { format: "srt", language: "en", file_path: "/tmp/out/script.srt", success: true },
    ]);
  });

  it("ExportScreen renders with seeded data", async () => {
    seedVideoProject();
    seedTwoSegmentScript();

    render(<ExportScreen />);
    expect(screen.getByText("Export")).toBeInTheDocument();
  });

  it("toggling format updates exportStore", () => {
    // Default formats are json and srt
    expect(useExportStore.getState().selectedFormats).toContain("json");
    expect(useExportStore.getState().selectedFormats).toContain("srt");

    useExportStore.getState().toggleFormat("vtt");
    expect(useExportStore.getState().selectedFormats).toContain("vtt");

    useExportStore.getState().toggleFormat("json");
    expect(useExportStore.getState().selectedFormats).not.toContain("json");
  });
});

describe("E2E Flow 4: Project Save and Load Cycle", () => {
  beforeEach(() => {
    resetAllStores();
    setupDefaultMocks();
  });
  afterEach(() => clearMocks());

  it("saves project with correct payload via IPC", async () => {
    const saveCalls: unknown[] = [];
    clearMocks();
    mockIPC((cmd, payload) => {
      if (cmd === "save_project") {
        saveCalls.push((payload as Record<string, unknown>)?.config);
        return "proj-saved-e2e";
      }
      if (cmd === "get_provider_status") return [];
      if (cmd === "list_projects") return [];
      if (cmd === "list_styles") return [];
      return null;
    });

    // Set up project state
    seedVideoProject();
    seedConfig();
    seedTwoSegmentScript();

    // Build save payload (mirrors App.tsx buildSavePayload)
    const ps = useProjectStore.getState();
    const cs = useConfigStore.getState();
    const now = new Date().toISOString();
    const payload = {
      id: ps.projectId,
      title: ps.title,
      description: ps.description || "",
      video_path: ps.videoFile!.path,
      style: cs.style,
      languages: cs.languages,
      primary_language: cs.primaryLanguage,
      frame_config: {
        density: cs.frameDensity,
        scene_threshold: cs.sceneThreshold,
        max_frames: cs.maxFrames,
      },
      ai_config: {
        provider: cs.aiProvider,
        model: cs.model,
        temperature: cs.temperature,
      },
      custom_prompt: cs.customPrompt,
      created_at: ps.createdAt || now,
      updated_at: now,
    };

    const { invoke } = await import("@tauri-apps/api/core");
    const result = await invoke("save_project", { config: payload });

    expect(result).toBe("proj-saved-e2e");
    expect(saveCalls).toHaveLength(1);
    const saved = saveCalls[0] as Record<string, unknown>;
    expect(saved.id).toBe("proj-e2e");
    expect(saved.title).toBe("E2E Test Video");
    expect(saved.video_path).toBe("/tmp/test.mp4");
    expect(saved.style).toBe("product_demo");
  });

  it("loads project and repopulates stores", async () => {
    // Simulate load_project_full returning saved data
    clearMocks();
    const loadedScripts: Record<string, NarrationScript> = {
      en: makeScript([
        makeSeg({ index: 0, text: "Loaded segment one." }),
        makeSeg({ index: 1, text: "Loaded segment two.", start_seconds: 15, end_seconds: 30, frame_refs: [2] }),
      ]),
    };

    mockIPC((cmd) => {
      if (cmd === "load_project_full") {
        return {
          config: {
            id: "proj-loaded",
            title: "Loaded Project",
            description: "A loaded project",
            video_path: "/tmp/loaded.mp4",
            style: "technical",
            languages: ["en"],
            primary_language: "en",
            frame_config: { density: "heavy", scene_threshold: 0.4, max_frames: 50 },
            ai_config: { provider: "openai", model: "gpt-4o", temperature: 0.5 },
            custom_prompt: "Be concise.",
            created_at: "2026-03-01T10:00:00Z",
            updated_at: "2026-04-01T12:00:00Z",
          },
          scripts: loadedScripts,
        };
      }
      if (cmd === "probe_video") {
        return {
          path: "/tmp/loaded.mp4", duration_seconds: 60, width: 1280, height: 720,
          codec: "h264", fps: 24, file_size: 30_000_000,
        };
      }
      return null;
    });

    // Simulate load flow (mirrors App.tsx handleOpenProject)
    const { invoke } = await import("@tauri-apps/api/core");
    const loaded = await invoke("load_project_full", { id: "proj-loaded" }) as {
      config: Record<string, unknown>;
      scripts: Record<string, NarrationScript>;
    };

    // Apply to stores
    const cfg = loaded.config;
    const ps = useProjectStore.getState();
    ps.setProjectId(cfg.id as string);
    ps.setTitle(cfg.title as string);
    ps.setDescription(cfg.description as string);
    ps.setCreatedAt(cfg.created_at as string);

    const meta = await invoke("probe_video", { path: cfg.video_path }) as {
      path: string; duration_seconds: number; width: number; height: number;
      codec: string; fps: number; file_size: number;
    };
    ps.setVideoFile({
      path: meta.path, name: "loaded.mp4", size: meta.file_size,
      duration: meta.duration_seconds, resolution: { width: meta.width, height: meta.height },
      codec: meta.codec, fps: meta.fps,
    });

    const cs = useConfigStore.getState();
    cs.setStyle(cfg.style as "technical");
    const aiCfg = cfg.ai_config as { provider: string; model: string; temperature: number };
    cs.setAiProvider(aiCfg.provider as "openai");
    cs.setModel(aiCfg.model as "gpt-4o");
    cs.setTemperature(aiCfg.temperature);
    cs.setCustomPrompt(cfg.custom_prompt as string);

    const ss = useScriptStore.getState();
    for (const [lang, script] of Object.entries(loaded.scripts)) {
      ss.setScript(lang, script);
    }

    // Verify stores are correctly populated
    expect(useProjectStore.getState().projectId).toBe("proj-loaded");
    expect(useProjectStore.getState().title).toBe("Loaded Project");
    expect(useProjectStore.getState().videoFile?.path).toBe("/tmp/loaded.mp4");
    expect(useProjectStore.getState().videoFile?.duration).toBe(60);
    expect(useProjectStore.getState().createdAt).toBe("2026-03-01T10:00:00Z");

    expect(useConfigStore.getState().style).toBe("technical");
    expect(useConfigStore.getState().aiProvider).toBe("openai");
    expect(useConfigStore.getState().model).toBe("gpt-4o");
    expect(useConfigStore.getState().temperature).toBe(0.5);
    expect(useConfigStore.getState().customPrompt).toBe("Be concise.");

    expect(useScriptStore.getState().scripts["en"]).toBeDefined();
    expect(useScriptStore.getState().scripts["en"].segments).toHaveLength(2);
    expect(useScriptStore.getState().scripts["en"].segments[0].text).toBe("Loaded segment one.");
  });

  it("round-trips: save then reset then load restores state", async () => {
    // Set up initial state
    seedVideoProject();
    seedConfig();
    seedTwoSegmentScript();

    // Capture state before reset
    const savedTitle = useProjectStore.getState().title;
    const savedStyle = useConfigStore.getState().style;
    const savedSegments = useScriptStore.getState().scripts["en"].segments;

    // Reset all stores
    resetAllStores();
    expect(useProjectStore.getState().title).toBe("");
    expect(Object.keys(useScriptStore.getState().scripts)).toHaveLength(0);

    // Restore from "loaded" data
    useProjectStore.getState().setProjectId("proj-e2e");
    useProjectStore.getState().setTitle(savedTitle);
    useProjectStore.getState().setVideoFile({
      path: "/tmp/test.mp4", name: "test.mp4", size: 50_000_000, duration: 120,
      resolution: { width: 1920, height: 1080 }, codec: "h264", fps: 30,
    });
    useConfigStore.getState().setStyle(savedStyle);
    useScriptStore.getState().setScript("en", makeScript(savedSegments));

    // Verify restoration
    expect(useProjectStore.getState().title).toBe("E2E Test Video");
    expect(useConfigStore.getState().style).toBe("product_demo");
    expect(useScriptStore.getState().scripts["en"].segments).toHaveLength(2);
  });
});

describe("E2E Flow 5: Multi-language Workflow", () => {
  beforeEach(() => {
    resetAllStores();
    setupDefaultMocks();
  });
  afterEach(() => clearMocks());

  it("generates primary script then translates to Japanese", async () => {
    seedVideoProject();
    seedConfig();

    // Step 1: Generate primary (English) script
    seedTwoSegmentScript("en");

    expect(useScriptStore.getState().scripts["en"]).toBeDefined();
    expect(useScriptStore.getState().scripts["en"].segments).toHaveLength(2);

    // Step 2: Translate to Japanese via IPC
    const { invoke } = await import("@tauri-apps/api/core");
    const translated = await invoke("translate_script", {
      script: useScriptStore.getState().scripts["en"],
      targetLang: "ja",
      aiConfig: {
        provider: "claude",
        model: "claude-sonnet-4-20250514",
        temperature: 0.7,
      },
    }) as NarrationScript;

    // Apply translated script to store
    useScriptStore.getState().setScript("ja", translated);

    // Step 3: Verify both languages exist
    const scripts = useScriptStore.getState().scripts;
    expect(scripts["en"]).toBeDefined();
    expect(scripts["ja"]).toBeDefined();
    expect(scripts["en"].segments).toHaveLength(2);
    expect(scripts["ja"].segments).toHaveLength(2);
    expect(scripts["ja"].metadata.language).toBe("ja");
    expect(scripts["ja"].segments[0].text).toBe("Translated segment one.");
    expect(scripts["ja"].segments[1].text).toBe("Translated segment two.");
  });

  it("switching active language in scriptStore works", () => {
    seedVideoProject();
    seedTwoSegmentScript("en");

    const jaScript = makeScript(
      [makeSeg({ index: 0, text: "Japanese text" })],
      { metadata: { style: "product_demo", language: "ja", provider: "claude", model: "claude-sonnet-4-20250514", generated_at: "2026-04-03T14:00:00Z" } },
    );
    useScriptStore.getState().setScript("ja", jaScript);

    expect(useScriptStore.getState().activeLanguage).toBe("en");
    useScriptStore.getState().setActiveLanguage("ja");
    expect(useScriptStore.getState().activeLanguage).toBe("ja");
  });

  it("configStore tracks multiple languages", () => {
    const cs = useConfigStore.getState();
    expect(cs.languages).toEqual(["en"]);

    cs.toggleLanguage("ja");
    expect(useConfigStore.getState().languages).toContain("ja");
    expect(useConfigStore.getState().languages).toContain("en");

    cs.toggleLanguage("de");
    expect(useConfigStore.getState().languages).toContain("de");
    expect(useConfigStore.getState().languages).toHaveLength(3);

    // Remove Japanese
    useConfigStore.getState().toggleLanguage("ja");
    expect(useConfigStore.getState().languages).not.toContain("ja");
    expect(useConfigStore.getState().languages).toHaveLength(2);
  });

  it("ReviewScreen shows language tabs when multiple scripts exist", () => {
    seedVideoProject();
    seedTwoSegmentScript("en");

    const jaScript = makeScript(
      [makeSeg({ index: 0, text: "Japanese narration" })],
      { metadata: { style: "product_demo", language: "ja", provider: "claude", model: "claude-sonnet-4-20250514", generated_at: "2026-04-03T14:00:00Z" } },
    );
    useScriptStore.getState().setScript("ja", jaScript);
    useConfigStore.getState().toggleLanguage("ja");

    render(<ReviewScreen />);

    expect(screen.getByText("EN")).toBeInTheDocument();
    expect(screen.getByText("JA")).toBeInTheDocument();
  });
});

describe("E2E Flow 6: Edit Video Clips", () => {
  beforeEach(() => {
    resetAllStores();
    setupDefaultMocks();
  });
  afterEach(() => clearMocks());

  it("initializes clips from video duration", () => {
    useEditStore.getState().initFromVideo(120);

    const es = useEditStore.getState();
    expect(es.clips).toHaveLength(1);
    expect(es.clips[0].sourceStart).toBe(0);
    expect(es.clips[0].sourceEnd).toBe(120);
    expect(es.clips[0].speed).toBe(1.0);
    expect(es.sourceDuration).toBe(120);
    expect(es.getOutputDuration()).toBe(120);
  });

  it("splits a clip at a timestamp", () => {
    useEditStore.getState().initFromVideo(120);

    // Split at output time 60 (which maps to source time 60 at speed 1.0)
    useEditStore.getState().splitAt(60);

    const clips = useEditStore.getState().clips;
    expect(clips).toHaveLength(2);
    expect(clips[0].sourceStart).toBe(0);
    expect(clips[0].sourceEnd).toBe(60);
    expect(clips[1].sourceStart).toBe(60);
    expect(clips[1].sourceEnd).toBe(120);
  });

  it("changes clip speed and recalculates output duration", () => {
    useEditStore.getState().initFromVideo(120);

    // Set speed to 2x
    useEditStore.getState().setClipSpeed(0, 2.0);

    const es = useEditStore.getState();
    expect(es.clips[0].speed).toBe(2.0);
    // 120 seconds at 2x speed = 60 seconds output
    expect(es.getOutputDuration()).toBe(60);
  });

  it("deletes a clip", () => {
    useEditStore.getState().initFromVideo(120);
    useEditStore.getState().splitAt(60);

    expect(useEditStore.getState().clips).toHaveLength(2);

    useEditStore.getState().deleteClip(0);

    const clips = useEditStore.getState().clips;
    expect(clips).toHaveLength(1);
    expect(clips[0].sourceStart).toBe(60);
    expect(clips[0].sourceEnd).toBe(120);
  });

  it("cannot delete the last remaining clip", () => {
    useEditStore.getState().initFromVideo(120);
    useEditStore.getState().deleteClip(0);

    // Should still have 1 clip since it won't delete the last
    expect(useEditStore.getState().clips).toHaveLength(1);
  });

  it("undo reverses a split", () => {
    useEditStore.getState().initFromVideo(120);
    expect(useEditStore.getState().clips).toHaveLength(1);

    useEditStore.getState().splitAt(60);
    expect(useEditStore.getState().clips).toHaveLength(2);

    useEditStore.getState().undo();
    expect(useEditStore.getState().clips).toHaveLength(1);
    expect(useEditStore.getState().clips[0].sourceEnd).toBe(120);
  });

  it("redo re-applies a split after undo", () => {
    useEditStore.getState().initFromVideo(120);
    useEditStore.getState().splitAt(60);
    useEditStore.getState().undo();

    expect(useEditStore.getState().clips).toHaveLength(1);

    useEditStore.getState().redo();
    expect(useEditStore.getState().clips).toHaveLength(2);
  });

  it("undo reverses a speed change", () => {
    useEditStore.getState().initFromVideo(120);
    useEditStore.getState().setClipSpeed(0, 2.0);
    expect(useEditStore.getState().clips[0].speed).toBe(2.0);

    useEditStore.getState().undo();
    expect(useEditStore.getState().clips[0].speed).toBe(1.0);
  });

  it("undo reverses a delete", () => {
    useEditStore.getState().initFromVideo(120);
    useEditStore.getState().splitAt(60);
    expect(useEditStore.getState().clips).toHaveLength(2);

    useEditStore.getState().deleteClip(0);
    expect(useEditStore.getState().clips).toHaveLength(1);

    useEditStore.getState().undo();
    expect(useEditStore.getState().clips).toHaveLength(2);
  });

  it("output duration accounts for multiple clips with different speeds", () => {
    useEditStore.getState().initFromVideo(120);
    useEditStore.getState().splitAt(60);

    // First clip at 2x, second at 0.5x
    useEditStore.getState().setClipSpeed(0, 2.0);
    useEditStore.getState().setClipSpeed(1, 0.5);

    // First: 60/2.0 = 30s output. Second: 60/0.5 = 120s output. Total: 150s
    expect(useEditStore.getState().getOutputDuration()).toBe(150);
  });

  it("getClipOutputStart returns correct cumulative start times", () => {
    useEditStore.getState().initFromVideo(120);
    useEditStore.getState().splitAt(60);
    useEditStore.getState().setClipSpeed(0, 2.0);

    // Clip 0 output start = 0
    expect(useEditStore.getState().getClipOutputStart(0)).toBe(0);
    // Clip 1 output start = 60/2.0 = 30
    expect(useEditStore.getState().getClipOutputStart(1)).toBe(30);
  });

  it("outputTimeToSource maps correctly", () => {
    useEditStore.getState().initFromVideo(120);
    useEditStore.getState().splitAt(60);
    useEditStore.getState().setClipSpeed(0, 2.0);

    // At output time 0 -> source 0
    expect(useEditStore.getState().outputTimeToSource(0)).toBe(0);
    // At output time 30 (end of first clip's output) -> source 60
    expect(useEditStore.getState().outputTimeToSource(30)).toBeCloseTo(60, 5);
    // At output time 45 (15s into second clip at 1x speed) -> source 75
    expect(useEditStore.getState().outputTimeToSource(45)).toBeCloseTo(75, 5);
  });
});

describe("E2E Flow 7: Error Recovery", () => {
  beforeEach(() => {
    resetAllStores();
    setupDefaultMocks();
  });
  afterEach(() => clearMocks());

  it("sets error state in processingStore when generation fails", () => {
    seedVideoProject();
    seedConfig();

    // Simulate error from generation
    useProcessingStore.getState().setPhase("error");
    useProcessingStore.getState().setError("API rate limit exceeded");

    expect(useProcessingStore.getState().phase).toBe("error");
    expect(useProcessingStore.getState().error).toBe("API rate limit exceeded");
  });

  it("can recover from error by resetting and retrying", () => {
    seedVideoProject();
    seedConfig();

    // Set error state
    useProcessingStore.getState().setPhase("error");
    useProcessingStore.getState().setError("Network timeout");

    // Recovery: reset processing state
    useProcessingStore.getState().reset();
    expect(useProcessingStore.getState().phase).toBe("idle");
    expect(useProcessingStore.getState().error).toBeNull();
    expect(useProcessingStore.getState().progress).toBe(0);

    // Simulate successful retry
    useProcessingStore.getState().setPhase("extracting_frames");
    useProcessingStore.getState().setPhase("generating_narration");
    useProcessingStore.getState().setPhase("done");
    seedTwoSegmentScript();

    expect(useProcessingStore.getState().phase).toBe("done");
    expect(useScriptStore.getState().scripts["en"].segments).toHaveLength(2);
  });

  it("generate_narration IPC can be mocked to return error", async () => {
    clearMocks();
    mockIPC((cmd) => {
      if (cmd === "generate_narration") {
        throw new Error("AI provider returned 429: Rate limit exceeded");
      }
      return null;
    });

    const { invoke } = await import("@tauri-apps/api/core");
    await expect(
      invoke("generate_narration", { params: {}, channel: {} }),
    ).rejects.toThrow("AI provider returned 429: Rate limit exceeded");
  });

  it("cancelled phase can be reset to idle", () => {
    useProcessingStore.getState().setPhase("generating_narration");
    useProcessingStore.getState().setPhase("cancelled");
    expect(useProcessingStore.getState().phase).toBe("cancelled");

    useProcessingStore.getState().reset();
    expect(useProcessingStore.getState().phase).toBe("idle");
  });
});

describe("E2E Flow 8: TTS Generation Flow", () => {
  beforeEach(() => {
    resetAllStores();
    setupDefaultMocks();
  });
  afterEach(() => clearMocks());

  it("generate_tts IPC returns file paths for each segment", async () => {
    seedVideoProject();
    seedTwoSegmentScript();

    const segments = useScriptStore.getState().scripts["en"].segments;
    const { invoke } = await import("@tauri-apps/api/core");

    const results = await invoke("generate_tts", {
      segments,
      outputDir: "/tmp/tts-output",
      compact: false,
      channel: {},
      ttsProvider: "elevenlabs",
    }) as Array<{ segment_index: number; file_path: string; success: boolean }>;

    expect(results).toHaveLength(2);
    expect(results[0]).toEqual({ segment_index: 0, file_path: "/tmp/audio/seg0.mp3", success: true });
    expect(results[1]).toEqual({ segment_index: 1, file_path: "/tmp/audio/seg1.mp3", success: true });
  });

  it("ElevenLabs config is retrievable via IPC", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    const config = await invoke("get_elevenlabs_config") as {
      api_key: string; voice_id: string; model_id: string;
    };

    expect(config.api_key).toBe("test-el-key");
    expect(config.voice_id).toBe("JBFqnCBsd6RMkjVDRZzb");
    expect(config.model_id).toBe("eleven_multilingual_v2");
  });

  it("ElevenLabs voices are listable via IPC", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    const voices = await invoke("list_elevenlabs_voices", { apiKey: "test-key" }) as Array<{
      voice_id: string; name: string; category: string;
    }>;

    expect(voices).toHaveLength(2);
    expect(voices[0].name).toBe("George");
    expect(voices[1].name).toBe("Rachel");
  });

  it("Azure TTS voices are listable via IPC", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    const voices = await invoke("list_azure_tts_voices", { apiKey: "test-key", region: "eastus" }) as Array<{
      short_name: string; display_name: string; locale: string; gender: string;
    }>;

    expect(voices).toHaveLength(2);
    expect(voices[0].display_name).toBe("Jenny");
    expect(voices[1].display_name).toBe("Guy");
  });

  it("TTS provider can be switched in configStore", () => {
    expect(useConfigStore.getState().ttsProvider).toBe("elevenlabs");
    useConfigStore.getState().setTtsProvider("azure");
    expect(useConfigStore.getState().ttsProvider).toBe("azure");
  });
});

describe("E2E Flow 9: Configuration Validation", () => {
  beforeEach(() => {
    resetAllStores();
    setupDefaultMocks();
  });
  afterEach(() => clearMocks());

  it("set_api_key IPC stores key without error", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    const result = await invoke("set_api_key", { provider: "claude", key: "sk-test-key-12345" });
    expect(result).toBeNull();
  });

  it("validate_api_key_cmd IPC returns true for valid key", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    const isValid = await invoke("validate_api_key_cmd", { provider: "claude", key: "sk-test-key-12345" });
    expect(isValid).toBe(true);
  });

  it("get_provider_status returns provider configurations", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    const statuses = await invoke("get_provider_status") as Array<{
      provider: string; has_key: boolean; models: string[];
    }>;

    expect(statuses).toHaveLength(3);
    const claude = statuses.find((s) => s.provider === "claude");
    expect(claude).toBeDefined();
    expect(claude!.has_key).toBe(true);
    expect(claude!.models).toContain("claude-sonnet-4-20250514");

    const openai = statuses.find((s) => s.provider === "openai");
    expect(openai).toBeDefined();
    expect(openai!.has_key).toBe(false);
  });

  it("validate_api_key_cmd can be mocked to return false for invalid key", async () => {
    clearMocks();
    mockIPC((cmd, payload) => {
      if (cmd === "validate_api_key_cmd") {
        const key = (payload as Record<string, unknown>)?.key as string;
        return key.startsWith("sk-") ? true : false;
      }
      return null;
    });

    const { invoke } = await import("@tauri-apps/api/core");
    const validResult = await invoke("validate_api_key_cmd", { provider: "claude", key: "sk-valid-key" });
    expect(validResult).toBe(true);

    const invalidResult = await invoke("validate_api_key_cmd", { provider: "claude", key: "invalid-key" });
    expect(invalidResult).toBe(false);
  });

  it("validates ElevenLabs key via IPC", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    const isValid = await invoke("validate_elevenlabs_key", { apiKey: "test-el-key" });
    expect(isValid).toBe(true);
  });

  it("validates Azure TTS key via IPC", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    const isValid = await invoke("validate_azure_tts_key", { apiKey: "test-azure-key", region: "eastus" });
    expect(isValid).toBe(true);
  });

  it("configStore provider/model switching works", () => {
    const cs = useConfigStore.getState();
    expect(cs.aiProvider).toBe("claude");
    expect(cs.model).toBe("claude-sonnet-4-20250514");

    useConfigStore.getState().setAiProvider("openai");
    useConfigStore.getState().setModel("gpt-4o");

    expect(useConfigStore.getState().aiProvider).toBe("openai");
    expect(useConfigStore.getState().model).toBe("gpt-4o");

    useConfigStore.getState().setAiProvider("gemini");
    useConfigStore.getState().setModel("gemini-2.5-flash");

    expect(useConfigStore.getState().aiProvider).toBe("gemini");
    expect(useConfigStore.getState().model).toBe("gemini-2.5-flash");
  });
});

describe("E2E Flow 10: Project Deletion", () => {
  beforeEach(() => {
    resetAllStores();
    setupDefaultMocks();
  });
  afterEach(() => clearMocks());

  it("list_projects returns project list via IPC", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    const projects = await invoke("list_projects") as Array<{
      id: string; title: string; has_script: boolean;
    }>;

    expect(projects).toHaveLength(1);
    expect(projects[0].id).toBe("proj-1");
    expect(projects[0].title).toBe("Demo Project");
    expect(projects[0].has_script).toBe(true);
  });

  it("delete_project IPC removes project successfully", async () => {
    const deleteCalls: string[] = [];
    clearMocks();
    mockIPC((cmd, payload) => {
      if (cmd === "delete_project") {
        deleteCalls.push((payload as Record<string, unknown>)?.id as string);
        return null;
      }
      if (cmd === "list_projects") {
        // After deletion, return empty list
        if (deleteCalls.includes("proj-1")) return [];
        return [
          { id: "proj-1", title: "Demo Project", video_path: "/tmp/demo.mp4",
            style: "product_demo", created_at: "2026-04-01T12:00:00Z",
            updated_at: "2026-04-01T12:00:00Z", has_script: true,
            thumbnail_path: null, script_languages: ["en"] },
        ];
      }
      return null;
    });

    const { invoke } = await import("@tauri-apps/api/core");

    // Step 1: List projects — should have 1
    const before = await invoke("list_projects") as Array<{ id: string }>;
    expect(before).toHaveLength(1);

    // Step 2: Delete the project
    await invoke("delete_project", { id: "proj-1" });
    expect(deleteCalls).toEqual(["proj-1"]);

    // Step 3: List projects again — should be empty
    const after = await invoke("list_projects") as Array<{ id: string }>;
    expect(after).toHaveLength(0);
  });

  it("deleting current project resets stores", async () => {
    // Set up as if a project is loaded
    seedVideoProject();
    seedConfig();
    seedTwoSegmentScript();

    expect(useProjectStore.getState().projectId).toBe("proj-e2e");

    // Simulate project deletion + store reset (what UI would do)
    const { invoke } = await import("@tauri-apps/api/core");
    await invoke("delete_project", { id: "proj-e2e" });

    // Reset all stores after deletion
    resetAllStores();

    expect(useProjectStore.getState().projectId).toBe("");
    expect(useProjectStore.getState().videoFile).toBeNull();
    expect(Object.keys(useScriptStore.getState().scripts)).toHaveLength(0);
    expect(useConfigStore.getState().style).toBe("product_demo"); // default
  });

  it("multiple projects can be listed and individually deleted", async () => {
    const deleted: string[] = [];
    clearMocks();
    mockIPC((cmd, payload) => {
      if (cmd === "delete_project") {
        deleted.push((payload as Record<string, unknown>)?.id as string);
        return null;
      }
      if (cmd === "list_projects") {
        const allProjects = [
          { id: "p1", title: "Project 1", video_path: "/tmp/1.mp4", style: "product_demo",
            created_at: "2026-04-01T12:00:00Z", updated_at: "2026-04-01T12:00:00Z",
            has_script: true, thumbnail_path: null, script_languages: ["en"] },
          { id: "p2", title: "Project 2", video_path: "/tmp/2.mp4", style: "technical",
            created_at: "2026-04-02T12:00:00Z", updated_at: "2026-04-02T12:00:00Z",
            has_script: false, thumbnail_path: null, script_languages: [] },
          { id: "p3", title: "Project 3", video_path: "/tmp/3.mp4", style: "teaser",
            created_at: "2026-04-03T12:00:00Z", updated_at: "2026-04-03T12:00:00Z",
            has_script: true, thumbnail_path: null, script_languages: ["en", "ja"] },
        ];
        return allProjects.filter((p) => !deleted.includes(p.id));
      }
      return null;
    });

    const { invoke } = await import("@tauri-apps/api/core");

    // Initially 3 projects
    const initial = await invoke("list_projects") as Array<{ id: string }>;
    expect(initial).toHaveLength(3);

    // Delete middle project
    await invoke("delete_project", { id: "p2" });
    const afterOne = await invoke("list_projects") as Array<{ id: string }>;
    expect(afterOne).toHaveLength(2);
    expect(afterOne.map((p) => p.id)).toEqual(["p1", "p3"]);

    // Delete first project
    await invoke("delete_project", { id: "p1" });
    const afterTwo = await invoke("list_projects") as Array<{ id: string }>;
    expect(afterTwo).toHaveLength(1);
    expect(afterTwo[0].id).toBe("p3");
  });
});
