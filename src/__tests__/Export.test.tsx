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

  it("renders the Export heading", () => {
    render(<ExportScreen />);
    expect(screen.getByText("Export")).toBeInTheDocument();
  });

  it("renders VIDEO, AUDIO ONLY, and SCRIPTS sections", () => {
    render(<ExportScreen />);
    expect(screen.getByText("VIDEO")).toBeInTheDocument();
    expect(screen.getByText("AUDIO ONLY")).toBeInTheDocument();
    expect(screen.getByText("SCRIPTS")).toBeInTheDocument();
  });

  it("shows default output directory after effect runs", async () => {
    render(<ExportScreen />);
    await screen.findByText(/Narrator/);
    expect(useExportStore.getState().outputDirectory).toContain("Narrator");
  });

  it("shows basename derived from project title", async () => {
    render(<ExportScreen />);
    await screen.findByText(/Narrator/);
    expect(useExportStore.getState().basename).toBe("test-project");
  });

  it("shows Export Video button when ElevenLabs configured", async () => {
    render(<ExportScreen />);
    await screen.findByText("Export Video");
    expect(screen.getByText("Export Video")).toBeInTheDocument();
  });

  it("shows Export Audio button", async () => {
    render(<ExportScreen />);
    await screen.findByText("Export Audio");
    expect(screen.getByText("Export Audio")).toBeInTheDocument();
  });

  it("shows script format buttons when SCRIPTS section is expanded", async () => {
    render(<ExportScreen />);
    const user = userEvent.setup();

    // SCRIPTS section is collapsed by default, click to expand
    await user.click(screen.getByText("SCRIPTS"));

    expect(screen.getByText("JSON")).toBeInTheDocument();
    expect(screen.getByText("SRT")).toBeInTheDocument();
    expect(screen.getByText("WebVTT")).toBeInTheDocument();
  });

  it("clicking format button toggles selection in exportStore", async () => {
    render(<ExportScreen />);
    const user = userEvent.setup();

    // Expand SCRIPTS section
    await user.click(screen.getByText("SCRIPTS"));

    expect(useExportStore.getState().selectedFormats).toContain("json");
    expect(useExportStore.getState().selectedFormats).toContain("srt");

    // Toggle off json
    await user.click(screen.getByText("JSON"));
    expect(useExportStore.getState().selectedFormats).not.toContain("json");
  });

  it("shows Copy and Export Scripts buttons in SCRIPTS section", async () => {
    render(<ExportScreen />);
    const user = userEvent.setup();

    await user.click(screen.getByText("SCRIPTS"));

    expect(screen.getByText("Copy")).toBeInTheDocument();
    expect(screen.getByText("Export Scripts")).toBeInTheDocument();
  });

  it("has subtitle toggle in VIDEO section", async () => {
    render(<ExportScreen />);
    await screen.findByText("Burn subtitles into video");
    expect(screen.getByText("Burn subtitles into video")).toBeInTheDocument();
  });

  it("shows voice summary card with Change button", async () => {
    render(<ExportScreen />);
    await screen.findByText("Export Video");

    // Should show voice summary (ElevenLabs from mock)
    expect(screen.getByText(/ElevenLabs/)).toBeInTheDocument();
    // There are two "Change" buttons (folder path + voice), ensure at least 2 exist
    const changeButtons = screen.getAllByText("Change");
    expect(changeButtons.length).toBeGreaterThanOrEqual(2);
  });
});
