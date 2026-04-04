import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { clearMocks } from "@tauri-apps/api/mocks";
import { setupDefaultMocks, resetAllStores } from "./setup";
import { ExportScreen } from "../features/export/ExportScreen";
import { useExportStore } from "../stores/exportStore";
import { useScriptStore } from "../stores/scriptStore";
import { useProjectStore } from "../stores/projectStore";
import type { NarrationScript } from "../types/script";

function seedScript() {
  const script: NarrationScript = {
    title: "Test Script",
    total_duration_seconds: 30,
    segments: [
      {
        index: 0,
        start_seconds: 0,
        end_seconds: 15,
        text: "First segment text.",
        visual_description: "Opening scene",
        emphasis: [],
        pace: "medium",
        pause_after_ms: 500,
        frame_refs: [0],
      },
      {
        index: 1,
        start_seconds: 15,
        end_seconds: 30,
        text: "Second segment text.",
        visual_description: "Main content",
        emphasis: [],
        pace: "medium",
        pause_after_ms: 0,
        frame_refs: [1],
      },
    ],
    metadata: {
      style: "product_demo",
      language: "en",
      provider: "claude",
      model: "claude-sonnet-4-20250514",
      generated_at: "2026-04-03T14:00:00Z",
    },
  };
  useScriptStore.getState().setScript("en", script);
}

describe("ExportScreen", () => {
  beforeEach(() => {
    resetAllStores();
    setupDefaultMocks();
    seedScript();
    useProjectStore.getState().setTitle("Test Project");
    useProjectStore.getState().setVideoFile({
      path: "/tmp/test.mp4",
      name: "test.mp4",
      size: 50_000_000,
      duration: 30,
      resolution: { width: 1920, height: 1080 },
      codec: "h264",
      fps: 30,
    });
  });

  afterEach(() => {
    clearMocks();
  });

  it("renders export format buttons", () => {
    render(<ExportScreen />);

    expect(screen.getByText("JSON (Structured)")).toBeInTheDocument();
    expect(screen.getByText("SRT (Subtitles)")).toBeInTheDocument();
    expect(screen.getByText("WebVTT")).toBeInTheDocument();
    expect(screen.getByText("Plain Text")).toBeInTheDocument();
    expect(screen.getByText("Markdown")).toBeInTheDocument();
    expect(screen.getByText("SSML (Speech)")).toBeInTheDocument();
  });

  it("clicking format button toggles selection in exportStore", async () => {
    render(<ExportScreen />);
    const user = userEvent.setup();

    // Default selected formats: json, srt
    expect(useExportStore.getState().selectedFormats).toContain("json");
    expect(useExportStore.getState().selectedFormats).toContain("srt");

    // Toggle off json
    await user.click(screen.getByText("JSON (Structured)"));
    expect(useExportStore.getState().selectedFormats).not.toContain("json");

    // Toggle on WebVTT
    await user.click(screen.getByText("WebVTT"));
    expect(useExportStore.getState().selectedFormats).toContain("vtt");
  });

  it("shows default output directory after effect runs", async () => {
    render(<ExportScreen />);

    // The component sets a default output directory via useEffect + getHomeDir IPC
    // After render, the export store should have an output directory set
    // Wait for the effect to run
    await screen.findByText(/Narrator/);
    expect(useExportStore.getState().outputDirectory).toContain("Narrator");
  });

  it("shows Generate Audio section when ElevenLabs configured", async () => {
    render(<ExportScreen />);

    // The component loads ElevenLabs config via IPC on mount.
    // Our mock returns a config with an api_key, so TTS UI should appear.
    await screen.findByText("Generate Audio");
    expect(screen.getByText("Generate Audio")).toBeInTheDocument();
  });

  it("shows Final Video section", () => {
    render(<ExportScreen />);

    expect(screen.getByText("Final Video")).toBeInTheDocument();
  });

  it("shows the Scripts section label", () => {
    render(<ExportScreen />);

    expect(screen.getByText("Scripts")).toBeInTheDocument();
  });

  it("shows Export and Copy buttons", () => {
    render(<ExportScreen />);

    // "Export" appears as both heading and button; use getAllByText
    const exportElements = screen.getAllByText("Export");
    expect(exportElements.length).toBeGreaterThanOrEqual(2); // heading + button
    expect(screen.getByText("Copy")).toBeInTheDocument();
  });
});
