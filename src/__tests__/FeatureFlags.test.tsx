import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";
import { resetAllStores } from "./setup";
import { ProcessingScreen } from "../features/processing/ProcessingScreen";
import { ConfigurationScreen } from "../features/configuration/ConfigurationScreen";
import { useProjectStore } from "../stores/projectStore";
import { useConfigStore } from "../stores/configStore";
import type { GenerationParams } from "../types/config";

/**
 * The 0.10 grounding features are only useful if the flags actually reach the
 * backend. The store defaults are covered in `configStore.test.ts`; these tests
 * cover the wiring — a default that never gets sent is indistinguishable from a
 * feature that was never enabled.
 */

/** Captures the params `generate_narration` was invoked with. */
function mockWithCapture(): { params: () => GenerationParams | null } {
  let captured: GenerationParams | null = null;

  mockIPC((cmd, payload) => {
    switch (cmd) {
      case "generate_narration": {
        captured = (payload as { params: GenerationParams }).params;
        return {
          title: "Captured",
          total_duration_seconds: 10,
          segments: [
            {
              index: 0,
              start_seconds: 0,
              end_seconds: 10,
              text: "Text.",
              visual_description: "",
              emphasis: [],
              pace: "medium",
              pause_after_ms: 0,
              frame_refs: [],
            },
          ],
          metadata: {
            style: "product_demo",
            language: "en",
            provider: "claude",
            model: "claude-sonnet-5",
            generated_at: "2026-01-01T00:00:00Z",
          },
        };
      }
      case "get_provider_status":
        return [{ provider: "claude", has_key: true, models: ["claude-sonnet-5"] }];
      case "list_styles":
        return [
          {
            id: "product_demo",
            label: "Product Demo",
            description: "Polished walkthrough",
            system_prompt: "",
            pacing: "medium",
            pause_markers: false,
          },
        ];
      case "get_home_dir":
        return "/Users/test";
      case "file_exists":
        return false;
      case "track_event":
      case "get_telemetry_enabled":
        return true;
      default:
        return null;
    }
  });

  return { params: () => captured };
}

function seedProjectWithVideo() {
  useProjectStore.setState({
    projectId: "22222222-2222-4222-8222-222222222222",
    title: "Flag Test",
    description: "",
    videoFile: {
      path: "/tmp/video.mp4",
      name: "video.mp4",
      size: 1000,
      duration: 30,
      resolution: { width: 1920, height: 1080 },
      codec: "h264",
      fps: 30,
    },
  } as Partial<ReturnType<typeof useProjectStore.getState>> as never);
}

async function startGenerationViaUi() {
  const user = userEvent.setup();
  render(<ProcessingScreen />);
  const start = await screen.findByText("Start Generation");
  await user.click(start);
}

describe("0.10 feature flags reach the backend", () => {
  beforeEach(() => {
    resetAllStores();
    seedProjectWithVideo();
  });

  afterEach(() => {
    clearMocks();
  });

  it("sends the grounding flags enabled by default", async () => {
    const capture = mockWithCapture();
    await startGenerationViaUi();

    await waitFor(() => expect(capture.params()).not.toBeNull());
    const params = capture.params()!;

    // These are the two features enabled in 0.10.
    expect(params.use_screen_text).toBe(true);
    expect(params.use_model_frame_selection).toBe(true);
  });

  it("does not enable contact sheets", async () => {
    // Deliberately held back: tiling halves the resolution the model sees, and
    // OCR only compensates on platforms with a recognizer.
    const capture = mockWithCapture();
    await startGenerationViaUi();

    await waitFor(() => expect(capture.params()).not.toBeNull());
    const params = capture.params()!;
    expect(params.use_contact_sheets).toBeFalsy();
  });

  it("honours the user turning a flag off", async () => {
    // A toggle that the request ignores is worse than no toggle.
    useConfigStore.getState().setScreenText(false);
    useConfigStore.getState().setModelFrameSelection(false);

    const capture = mockWithCapture();
    await startGenerationViaUi();

    await waitFor(() => expect(capture.params()).not.toBeNull());
    const params = capture.params()!;
    expect(params.use_screen_text).toBe(false);
    expect(params.use_model_frame_selection).toBe(false);
  });

  it("keeps strict mode off unless asked", async () => {
    // Unchanged by this release — it costs up to 12 extra API calls.
    const capture = mockWithCapture();
    await startGenerationViaUi();

    await waitFor(() => expect(capture.params()).not.toBeNull());
    expect(capture.params()!.strict_mode).toBe(false);
  });
});

describe("Configuration screen exposes the new features", () => {
  beforeEach(() => {
    resetAllStores();
    mockWithCapture();
  });

  afterEach(() => {
    clearMocks();
  });

  it("renders both grounding toggles, checked by default", async () => {
    render(<ConfigurationScreen />);

    const screenText = await screen.findByText("Read on-screen text");
    const framePick = await screen.findByText("Let AI pick key moments");
    expect(screenText).toBeTruthy();
    expect(framePick).toBeTruthy();

    // Both must render as checked, or the UI would contradict the actual
    // request being sent.
    const boxes = screen
      .getAllByRole("checkbox")
      .filter((el) => (el as HTMLInputElement).checked);
    expect(boxes.length).toBeGreaterThanOrEqual(2);
  });

  it("clicking a toggle updates the store", async () => {
    const user = userEvent.setup();
    render(<ConfigurationScreen />);

    const label = await screen.findByText("Read on-screen text");
    // The checkbox is the input inside the same <label> wrapper.
    const wrapper = label.closest("label");
    expect(wrapper).toBeTruthy();
    const box = wrapper!.querySelector('input[type="checkbox"]') as HTMLInputElement;
    expect(box.checked).toBe(true);

    await user.click(box);
    expect(useConfigStore.getState().screenText).toBe(false);
  });

  it("discloses that on-screen text is macOS-only", async () => {
    // Users on Windows should not be left wondering why it does nothing.
    render(<ConfigurationScreen />);
    const copy = await screen.findByText(/macOS only/i);
    expect(copy).toBeTruthy();
  });
});
