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

    // Delete the first segment — opens confirmation dialog
    await user.click(delButtons[0]);

    // Confirm deletion in the dialog
    const confirmBtn = screen.getByText("Delete");
    await user.click(confirmBtn);

    // Store should now have 1 segment
    const segments = useScriptStore.getState().scripts["en"].segments;
    expect(segments).toHaveLength(1);
    expect(segments[0].text).toBe("Here we see the main feature.");
    expect(segments[0].index).toBe(0); // Re-indexed
  });

  it("renders editable textareas for each segment", () => {
    seedStores();
    render(<ReviewScreen />);

    const editButtons = screen.getAllByRole("textbox");
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
    expect(screen.getByText("No narration generated yet")).toBeInTheDocument();
  });

  // ── Preview (Play/Stop) button tests ──

  it("renders Play button for each segment", () => {
    seedStores();
    render(<ReviewScreen />);

    const playButtons = screen.getAllByText(/Play/);
    expect(playButtons).toHaveLength(2);
  });

  it("clicking Play button changes it to Stop state", async () => {
    seedStores();
    const user = userEvent.setup();
    render(<ReviewScreen />);

    const playButtons = screen.getAllByText(/Play/);
    expect(playButtons[0].textContent).toContain("Play");

    // Click the first Play button — it will try to generate TTS (mocked) and play audio.
    // In jsdom audio won't actually play, but the state should change to "previewing"
    // which shows the Stop text. The handlePreview sets previewingIdx immediately.
    await user.click(playButtons[0]);

    // After clicking, the button for that segment should show Stop
    // (the mock IPC returns a TTS result, but jsdom's Audio won't fire events,
    // so previewingIdx stays set until audio ends or errors)
    const stopButtons = screen.queryAllByText(/Stop/);
    expect(stopButtons.length).toBeGreaterThanOrEqual(1);
  });

  it("renders AI refine button per segment", () => {
    seedStores();
    render(<ReviewScreen />);

    const aiButtons = screen.getAllByText("AI");
    expect(aiButtons).toHaveLength(2);
  });

  it("AI refine dropdown shows presets when clicked", async () => {
    seedStores();
    const user = userEvent.setup();
    render(<ReviewScreen />);

    const aiButtons = screen.getAllByText("AI");
    await user.click(aiButtons[0]);

    expect(screen.getByText("Refine with AI")).toBeInTheDocument();
    expect(screen.getByText("Make shorter")).toBeInTheDocument();
    expect(screen.getByText("Make more detailed")).toBeInTheDocument();
    expect(screen.getByText("Simplify language")).toBeInTheDocument();
    expect(screen.getByText("More professional")).toBeInTheDocument();
    expect(screen.getByText("More conversational")).toBeInTheDocument();
    expect(screen.getByPlaceholderText("Custom instruction...")).toBeInTheDocument();
  });

  it("renders segment numbers", () => {
    seedStores();
    render(<ReviewScreen />);

    expect(screen.getByText("#1")).toBeInTheDocument();
    expect(screen.getByText("#2")).toBeInTheDocument();
  });

  it("renders Preview Narration button when segments exist", () => {
    seedStores();
    render(<ReviewScreen />);

    expect(screen.getByText(/Preview Narration/)).toBeInTheDocument();
  });

  it("renders voice picker button per segment", () => {
    seedStores();
    render(<ReviewScreen />);

    // Each segment has a "Voice" button (default = no override)
    const voiceButtons = screen.getAllByText("Voice");
    expect(voiceButtons.length).toBeGreaterThanOrEqual(2);
  });

  it("voice picker shows Project default option when clicked", async () => {
    seedStores();
    const user = userEvent.setup();
    render(<ReviewScreen />);

    const voiceButtons = screen.getAllByText("Voice");
    await user.click(voiceButtons[0]);

    expect(screen.getByText("Project default")).toBeInTheDocument();
  });
});
