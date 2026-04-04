import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { clearMocks } from "@tauri-apps/api/mocks";
import { setupDefaultMocks, resetAllStores } from "./setup";
import { ProcessingScreen } from "../features/processing/ProcessingScreen";
import { useScriptStore } from "../stores/scriptStore";
import { useProcessingStore } from "../stores/processingStore";
import { useProjectStore } from "../stores/projectStore";
import type { NarrationScript } from "../types/script";

function seedScriptAndProject() {
  const script: NarrationScript = {
    title: "Test Script",
    total_duration_seconds: 30,
    segments: [
      {
        index: 0,
        start_seconds: 0,
        end_seconds: 15,
        text: "Welcome to the demo.",
        visual_description: "Title screen",
        emphasis: [],
        pace: "medium",
        pause_after_ms: 500,
        frame_refs: [0, 1],
      },
      {
        index: 1,
        start_seconds: 15,
        end_seconds: 30,
        text: "Here we see the main feature.",
        visual_description: "Feature overview",
        emphasis: [],
        pace: "medium",
        pause_after_ms: 0,
        frame_refs: [2, 3],
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

  useProjectStore.getState().setVideoFile({
    path: "/tmp/test.mp4",
    name: "test.mp4",
    size: 50_000_000,
    duration: 30,
    resolution: { width: 1920, height: 1080 },
    codec: "h264",
    fps: 30,
  });
  useProjectStore.getState().setProjectId("proj-test");
  useProjectStore.getState().setTitle("Test");
}

describe("ProcessingScreen", () => {
  beforeEach(() => {
    resetAllStores();
    setupDefaultMocks();
  });

  afterEach(() => {
    clearMocks();
  });

  it("shows Generation complete state when scripts exist and phase is idle", () => {
    seedScriptAndProject();
    // Phase defaults to "idle", and scripts exist -> showCompleted = true
    render(<ProcessingScreen />);

    expect(screen.getByText("Generation complete!")).toBeInTheDocument();
    expect(screen.getByText("Narration Generated")).toBeInTheDocument();
  });

  it("shows Regenerate button when completed", () => {
    seedScriptAndProject();
    render(<ProcessingScreen />);

    expect(screen.getByText("Regenerate")).toBeInTheDocument();
  });

  it("shows segment count when completed", () => {
    seedScriptAndProject();
    render(<ProcessingScreen />);

    expect(screen.getByText("2 segments ready for review")).toBeInTheDocument();
  });

  it("shows completed state when phase is done and scripts exist", () => {
    seedScriptAndProject();
    useProcessingStore.getState().setPhase("done");

    render(<ProcessingScreen />);

    expect(screen.getByText("Generation complete!")).toBeInTheDocument();
    expect(screen.getByText("2 segments ready for review")).toBeInTheDocument();
  });

  it("shows Processing heading in all cases", () => {
    seedScriptAndProject();
    render(<ProcessingScreen />);

    expect(screen.getByText("Processing")).toBeInTheDocument();
  });
});
