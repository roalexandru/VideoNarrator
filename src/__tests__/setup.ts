import { vi } from "vitest";
import "@testing-library/jest-dom/vitest";
import { mockIPC } from "@tauri-apps/api/mocks";

// Force production-mode env so tests exercise prod code paths (auto-update check,
// plain version string). Individual tests can `vi.stubEnv("DEV", true)` to cover dev.
vi.stubEnv("DEV", "");
vi.stubEnv("PROD", "1");

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
  message: vi.fn(),
}));

// ---- Mock Tauri dialog plugin ----
vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: vi.fn(() => Promise.resolve("/tmp/test.narrator")),
  open: vi.fn(() => Promise.resolve("/tmp/test.narrator")),
  message: vi.fn(),
  ask: vi.fn(() => Promise.resolve(true)),
}));

// ---- Mock Tauri updater plugin ----
vi.mock("@tauri-apps/plugin-updater", () => ({
  check: vi.fn(),
}));

// ---- Mock Tauri process plugin ----
vi.mock("@tauri-apps/plugin-process", () => ({
  relaunch: vi.fn(),
}));

// ---- Mock Tauri event API ----
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
  emit: vi.fn(),
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
          { provider: "gemini", has_key: false, models: ["gemini-2.5-flash"] },
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

      case "get_azure_tts_config":
        return null;

      case "save_azure_tts_config":
        return null;

      case "list_azure_tts_voices":
        return [
          { short_name: "en-US-JennyNeural", display_name: "Jenny", locale: "en-US", gender: "Female" },
          { short_name: "en-US-GuyNeural", display_name: "Guy", locale: "en-US", gender: "Male" },
        ];

      case "validate_azure_tts_key":
        return true;

      case "list_builtin_voices":
        return [
          { id: "default", name: "System Default", locale: "en-US" },
        ];

      case "validate_elevenlabs_key":
        return true;

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

      case "extract_single_frame":
        return "/tmp/frame.jpg";

      case "save_script":
        return "/tmp/script.json";

      case "apply_video_edits":
        return "/tmp/edited.mp4";

      case "merge_audio_video":
        return "/tmp/final.mp4";

      case "burn_subtitles":
        return "/tmp/subtitled.mp4";

      case "open_folder":
        return null;

      case "record_screen_native":
        return "/Users/test/Documents/Narrator/Recordings/test-id.mov";

      case "set_menu_context":
        return null;

      case "get_telemetry_enabled":
        return true;

      case "set_telemetry_enabled":
        return null;

      case "track_event":
        return null;

      case "process_documents":
        return [{ name: "doc.pdf", content: "Sample document text", token_estimate: 150 }];

      case "translate_script":
        return {
          title: "Test Narration",
          total_duration_seconds: 30,
          segments: [
            {
              index: 0, start_seconds: 0, end_seconds: 15,
              text: "Translated segment one.", visual_description: "Title screen",
              emphasis: [], pace: "medium", pause_after_ms: 500, frame_refs: [0, 1],
            },
            {
              index: 1, start_seconds: 15, end_seconds: 30,
              text: "Translated segment two.", visual_description: "Feature overview",
              emphasis: [], pace: "medium", pause_after_ms: 0, frame_refs: [2, 3],
            },
          ],
          metadata: {
            style: "product_demo", language: "ja",
            provider: "claude", model: "claude-sonnet-4-20250514",
            generated_at: "2026-04-03T14:00:00Z",
          },
        };

      case "refine_segment":
        return "This is the refined segment text.";

      case "refine_script":
        return {
          title: "Refined Narration",
          total_duration_seconds: 30,
          segments: [
            { index: 0, start_seconds: 0, end_seconds: 15, text: "Refined opener.", visual_description: "Title screen", emphasis: [], pace: "medium", pause_after_ms: 500, frame_refs: [0, 1] },
            { index: 1, start_seconds: 15, end_seconds: 30, text: "Refined close.", visual_description: "Feature overview", emphasis: [], pace: "medium", pause_after_ms: 0, frame_refs: [2, 3] },
          ],
          metadata: { style: "product_demo", language: "en", provider: "claude", model: "claude-sonnet-4-20250514", generated_at: "2026-04-03T14:00:00Z" },
        };

      case "export_project":
        return null;

      case "import_project":
        return "proj-imported-1";

      case "save_template":
        return null;

      case "list_templates":
        return [
          {
            id: "tmpl-1", name: "Demo Template", style: "product_demo",
            languages: ["en"], primary_language: "en",
            frame_config: { density: "medium", scene_threshold: 0.3, max_frames: 30 },
            ai_config: { provider: "claude", model: "claude-sonnet-4-20250514", temperature: 0.7 },
            custom_prompt: "", tts_provider: "builtin", created_at: "2026-04-01T12:00:00Z",
          },
        ];

      case "delete_template":
        return null;

      case "cancel_generation":
        return null;

      case "start_screen_recording":
        return null;

      case "pause_recording":
        return null;

      case "resume_recording":
        return null;

      case "stop_screen_recording":
        return "/Users/test/Documents/Narrator/Recordings/test-id.mp4";

      case "get_recordings_directory":
        return "/Users/test/Documents/Narrator/Recordings";

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
