import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { clearMocks } from "@tauri-apps/api/mocks";
import { setupDefaultMocks, resetAllStores } from "./setup";
import { ReviewScreen } from "../features/review/ReviewScreen";
import { useScriptStore } from "../stores/scriptStore";
import { useProjectStore } from "../stores/projectStore";
import { useConfigStore } from "../stores/configStore";
import type { NarrationScript } from "../types/script";

function seedStores() {
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
        pace: "fast",
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
}

describe("ReviewScreen", () => {
  beforeEach(() => {
    resetAllStores();
    setupDefaultMocks();
  });

  afterEach(() => {
    clearMocks();
  });

  it("renders segment list with timestamps", () => {
    seedStores();
    render(<ReviewScreen />);

    // Timestamps may appear in both the timeline and the segment editor, so use getAllByText
    expect(screen.getAllByText("0:00").length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText("0:15").length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText("0:30").length).toBeGreaterThanOrEqual(1);
  });

  it("renders segment text", () => {
    seedStores();
    render(<ReviewScreen />);

    // Text may appear both in the segment list and the caption overlay
    expect(screen.getAllByText("Welcome to the demo.").length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText("Here we see the main feature.").length).toBeGreaterThanOrEqual(1);
  });

  it("renders segment pace badges", () => {
    seedStores();
    render(<ReviewScreen />);

    expect(screen.getAllByText("medium").length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText("fast").length).toBeGreaterThanOrEqual(1);
  });

  it("delete button removes segment from store", async () => {
    seedStores();
    const user = userEvent.setup();
    render(<ReviewScreen />);

    // There should be 2 "Del" buttons
    const delButtons = screen.getAllByText("Del");
    expect(delButtons).toHaveLength(2);

    // Delete the first segment
    await user.click(delButtons[0]);

    // Store should now have 1 segment
    const segments = useScriptStore.getState().scripts["en"].segments;
    expect(segments).toHaveLength(1);
    expect(segments[0].text).toBe("Here we see the main feature.");
    expect(segments[0].index).toBe(0); // Re-indexed
  });

  it("renders Edit buttons for each segment", () => {
    seedStores();
    render(<ReviewScreen />);

    const editButtons = screen.getAllByText("Edit");
    expect(editButtons).toHaveLength(2);
  });

  it("language tabs render when multiple languages configured", () => {
    seedStores();
    // Add a second language to config
    useConfigStore.getState().toggleLanguage("ja");

    // Add a Japanese script
    const jaScript: NarrationScript = {
      title: "Test Script",
      total_duration_seconds: 30,
      segments: [
        {
          index: 0,
          start_seconds: 0,
          end_seconds: 15,
          text: "Japanese text",
          visual_description: "",
          emphasis: [],
          pace: "medium",
          pause_after_ms: 0,
          frame_refs: [],
        },
      ],
      metadata: {
        style: "product_demo",
        language: "ja",
        provider: "claude",
        model: "claude-sonnet-4-20250514",
        generated_at: "2026-04-03T14:00:00Z",
      },
    };
    useScriptStore.getState().setScript("ja", jaScript);

    render(<ReviewScreen />);

    // Language tabs should appear
    expect(screen.getByText("EN")).toBeInTheDocument();
    expect(screen.getByText("JA")).toBeInTheDocument();
  });

  it("does not render language tabs when only one language", () => {
    seedStores();
    // Only "en" in config
    render(<ReviewScreen />);

    expect(screen.queryByText("EN")).not.toBeInTheDocument();
  });

  it("switching language tab changes displayed segments", async () => {
    seedStores();
    useConfigStore.getState().toggleLanguage("ja");

    const jaScript: NarrationScript = {
      title: "JA Script",
      total_duration_seconds: 20,
      segments: [
        {
          index: 0,
          start_seconds: 0,
          end_seconds: 20,
          text: "Japanese narration text",
          visual_description: "",
          emphasis: [],
          pace: "slow",
          pause_after_ms: 0,
          frame_refs: [],
        },
      ],
      metadata: {
        style: "product_demo",
        language: "ja",
        provider: "claude",
        model: "claude-sonnet-4-20250514",
        generated_at: "2026-04-03T14:00:00Z",
      },
    };
    useScriptStore.getState().setScript("ja", jaScript);

    const user = userEvent.setup();
    render(<ReviewScreen />);

    // Initially showing English (may appear in multiple places)
    expect(screen.getAllByText("Welcome to the demo.").length).toBeGreaterThanOrEqual(1);

    // Click JA tab
    await user.click(screen.getByText("JA"));

    // Should now show Japanese segments
    expect(screen.getAllByText("Japanese narration text").length).toBeGreaterThanOrEqual(1);
  });

  it("renders Review & Edit heading", () => {
    seedStores();
    render(<ReviewScreen />);
    expect(screen.getByText("Review & Edit")).toBeInTheDocument();
  });

  it("shows No segments message when script has no segments", () => {
    useProjectStore.getState().setVideoFile({
      path: "/tmp/test.mp4",
      name: "test.mp4",
      size: 50_000_000,
      duration: 30,
      resolution: { width: 1920, height: 1080 },
      codec: "h264",
      fps: 30,
    });
    // No script seeded

    render(<ReviewScreen />);
    expect(screen.getByText("No segments.")).toBeInTheDocument();
  });
});
