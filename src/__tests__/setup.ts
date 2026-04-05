import { vi } from "vitest";
import "@testing-library/jest-dom/vitest";
import { mockIPC } from "@tauri-apps/api/mocks";

// Polyfill scrollIntoView for jsdom (not implemented)
if (typeof Element.prototype.scrollIntoView !== "function") {
  Element.prototype.scrollIntoView = vi.fn();
}

import { useEditStore } from "../stores/editStore";
import { useProjectStore } from "../stores/projectStore";
import { useConfigStore } from "../stores/configStore";
import { useExportStore } from "../stores/exportStore";
import { useProcessingStore } from "../stores/processingStore";
import { useScriptStore } from "../stores/scriptStore";

// ---- Mock Tauri window API ----
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    minimize: vi.fn(),
    unminimize: vi.fn(),
    setFocus: vi.fn(),
    startDragging: vi.fn(),
    close: vi.fn(),
  }),
}));

// ---- Mock Tauri core (passthrough convertFileSrc) ----
vi.mock("@tauri-apps/api/core", async () => {
  const actual = await vi.importActual("@tauri-apps/api/core");
  return {
    ...actual,
    convertFileSrc: (p: string) => p,
  };
});

// ---- Mock Tauri dialog plugin ----
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
}));

/**
 * Sets up default mock IPC handlers for every known Tauri command.
 * Call this in `beforeEach` for component tests that trigger IPC.
 */
export function setupDefaultMocks() {
  mockIPC((cmd, payload) => {
    switch (cmd) {
      case "check_ffmpeg":
        return "/usr/local/bin/ffmpeg";

      case "get_provider_status":
        return [
          { provider: "claude", has_key: true, models: ["claude-sonnet-4-20250514"] },
          { provider: "openai", has_key: false, models: ["gpt-4o"] },
        ];

      case "probe_video":
        return {
          path: (payload as Record<string, unknown>)?.path ?? "/tmp/test.mp4",
          duration_seconds: 120,
          width: 1920,
          height: 1080,
          codec: "h264",
          fps: 30,
          file_size: 50_000_000,
        };

      case "list_projects":
        return [
          {
            id: "proj-1",
            title: "Demo Project",
            video_path: "/tmp/demo.mp4",
            style: "product_demo",
            created_at: "2026-04-01T12:00:00Z",
            updated_at: "2026-04-01T12:00:00Z",
            has_script: true,
            thumbnail_path: null,
            script_languages: ["en"],
          },
        ];

      case "list_styles":
        return [
          { id: "product_demo", label: "Product Demo", description: "Polished walkthrough", system_prompt: "", pacing: "medium", pause_markers: false },
          { id: "technical", label: "Technical Deep-Dive", description: "Developer-oriented", system_prompt: "", pacing: "medium", pause_markers: false },
        ];

      case "generate_narration":
        return {
          title: "Test Narration",
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

      case "export_script":
        return [
          { format: "json", language: "en", file_path: "/tmp/out/script.json", success: true },
          { format: "srt", language: "en", file_path: "/tmp/out/script.srt", success: true },
        ];

      case "generate_tts":
        return [
          { segment_index: 0, file_path: "/tmp/audio/seg0.mp3", success: true },
          { segment_index: 1, file_path: "/tmp/audio/seg1.mp3", success: true },
        ];

      case "get_elevenlabs_config":
        return {
          api_key: "test-el-key",
          voice_id: "JBFqnCBsd6RMkjVDRZzb",
          model_id: "eleven_multilingual_v2",
          stability: 0.5,
          similarity_boost: 0.75,
          style: 0,
          speed: 1.0,
        };

      case "save_elevenlabs_config":
        return null;

      case "list_elevenlabs_voices":
        return [
          { voice_id: "JBFqnCBsd6RMkjVDRZzb", name: "George", category: "premade" },
          { voice_id: "voice-2", name: "Rachel", category: "premade" },
        ];

      case "set_api_key":
        return null;

      case "validate_api_key_cmd":
        return true;

      case "save_project":
        return "proj-saved-1";

      case "load_project_full":
        return {
          config: {
            id: "proj-1",
            title: "Demo Project",
            description: "A demo",
            video_path: "/tmp/demo.mp4",
            style: "product_demo",
            languages: ["en"],
            primary_language: "en",
            frame_config: { density: "medium", scene_threshold: 0.3, max_frames: 30 },
            ai_config: { provider: "claude", model: "claude-sonnet-4-20250514", temperature: 0.7 },
            custom_prompt: "",
            created_at: "2026-04-01T12:00:00Z",
            updated_at: "2026-04-01T12:00:00Z",
          },
          scripts: {},
        };

      case "delete_project":
        return null;

      case "get_home_dir":
        return "/Users/testuser";

      case "list_project_frames":
        return [
          { index: 0, path: "/tmp/frames/frame_0.png" },
          { index: 1, path: "/tmp/frames/frame_1.png" },
        ];

      case "extract_edit_thumbnails":
        return ["/tmp/thumbs/t0.png", "/tmp/thumbs/t1.png"];

      case "apply_video_edits":
        return "/tmp/edited.mp4";

      case "merge_audio_video":
        return "/tmp/final.mp4";

      case "burn_subtitles":
        return "/tmp/subtitled.mp4";

      case "open_folder":
        return null;

      case "record_screen_native":
        return "/tmp/recording.mp4";

      case "set_menu_context":
        return null;

      case "get_telemetry_enabled":
        return true;

      case "set_telemetry_enabled":
        return null;

      case "track_event":
        return null;

      default:
        console.warn(`[mockIPC] Unhandled command: ${cmd}`);
        return null;
    }
  });
}

/**
 * Reset every Zustand store to its initial state.
 */
export function resetAllStores() {
  useEditStore.getState().reset();
  useProjectStore.getState().reset();
  useConfigStore.getState().reset();
  useExportStore.getState().reset();
  useProcessingStore.getState().reset();
  useScriptStore.getState().reset();
}
