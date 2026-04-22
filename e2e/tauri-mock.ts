import { Page } from "@playwright/test";

/**
 * Injects Tauri IPC mocks into the page before the app loads.
 * Must be called before page.goto().
 *
 * The Tauri v2 JS API calls window.__TAURI_INTERNALS__.invoke(cmd, args)
 * for all IPC — both custom commands and built-in plugin commands
 * (e.g. "plugin:event|listen", "plugin:updater|check", "plugin:dialog|open").
 *
 * It also uses transformCallback() to register JS callbacks that Rust
 * can later call back into, and metadata.currentWindow / currentWebview
 * for window/webview identity.
 */
export async function mockTauriIPC(page: Page, overrides?: Record<string, unknown>) {
  await page.addInitScript((overridesJson) => {
    const overrides = JSON.parse(overridesJson);

    // Default mock responses for all IPC commands
    const defaults: Record<string, unknown> = {
      check_ffmpeg: "/usr/local/bin/ffmpeg",
      ffmpeg_supports_subtitle_burn: true,
      get_provider_status: [
        { provider: "claude", has_key: true, models: ["claude-sonnet-4-20250514"] },
        { provider: "openai", has_key: false, models: ["gpt-4o"] },
        { provider: "gemini", has_key: false, models: ["gemini-2.5-flash"] },
      ],
      probe_video: {
        path: "/tmp/test.mp4",
        duration_seconds: 120,
        width: 1920,
        height: 1080,
        codec: "h264",
        fps: 30,
        file_size: 50000000,
      },
      list_projects: [
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
      ],
      list_styles: [
        { id: "executive", label: "Executive Overview", description: "Confident, outcome-focused", system_prompt: "", pacing: "medium", pause_markers: false },
        { id: "product_demo", label: "Product Demo", description: "Polished walkthrough", system_prompt: "", pacing: "medium", pause_markers: false },
        { id: "technical", label: "Technical Deep-Dive", description: "Developer-oriented", system_prompt: "", pacing: "medium", pause_markers: false },
        { id: "teaser", label: "Teaser / Trailer", description: "High-energy", system_prompt: "", pacing: "fast", pause_markers: false },
        { id: "training", label: "Training Walkthrough", description: "Patient, methodical", system_prompt: "", pacing: "slow", pause_markers: false },
        { id: "critique", label: "Bug Review / Critique", description: "Analytical review", system_prompt: "", pacing: "medium", pause_markers: false },
      ],
      get_elevenlabs_config: {
        api_key: "test-el-key",
        voice_id: "JBFqnCBsd6RMkjVDRZzb",
        model_id: "eleven_multilingual_v2",
        stability: 0.5,
        similarity_boost: 0.75,
        style: 0,
        speed: 1.0,
      },
      get_azure_tts_config: null,
      save_elevenlabs_config: null,
      save_azure_tts_config: null,
      list_elevenlabs_voices: [
        { voice_id: "JBFqnCBsd6RMkjVDRZzb", name: "George", category: "premade" },
        { voice_id: "voice-2", name: "Rachel", category: "premade" },
      ],
      list_azure_tts_voices: [
        { short_name: "en-US-JennyNeural", display_name: "Jenny", locale: "en-US", gender: "Female" },
      ],
      get_home_dir: "/Users/testuser",
      set_api_key: null,
      validate_api_key_cmd: true,
      validate_elevenlabs_key: true,
      validate_azure_tts_key: true,
      save_project: "proj-saved-1",
      load_project_full: {
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
      },
      delete_project: null,
      list_project_frames: [],
      extract_edit_thumbnails: [],
      open_folder: null,
      get_telemetry_enabled: false,
      set_telemetry_enabled: null,
      track_event: null,
      cancel_generation: null,
      export_script: [
        { format: "json", language: "en", file_path: "/tmp/out/script.json", success: true, error: null },
      ],
      set_menu_context: null,
      process_documents: [],
      generate_tts: [
        { segment_index: 0, file_path: "/tmp/audio/seg0.mp3", success: true, error: null },
      ],
      burn_subtitles: "/tmp/subtitled.mp4",
      merge_audio_video: "/tmp/final.mp4",
      apply_video_edits: "/tmp/edited.mp4",
      get_recordings_directory: "/Users/test/Documents/Narrator/Recordings",
    };

    const responses: Record<string, unknown> = { ...defaults, ...overrides };

    // ── Callback registry (mirrors Tauri's internal callback system) ──
    let nextCallbackId = 1;
    const callbacks: Record<number, Function> = {};

    function transformCallback(callback?: Function, once = false): number {
      const id = nextCallbackId++;
      if (callback) {
        callbacks[id] = once
          ? (...args: unknown[]) => { callback(...args); delete callbacks[id]; }
          : callback;
      }
      return id;
    }

    function unregisterCallback(id: number) {
      delete callbacks[id];
    }

    // ── IPC invoke mock ──
    function invoke(cmd: string, args?: any): Promise<unknown> {
      console.log(`[Tauri Mock] invoke: ${cmd}`, args);

      // Handle plugin commands (e.g. "plugin:event|listen", "plugin:updater|check")
      if (cmd.startsWith("plugin:")) {
        // plugin:event|listen — return a fake event ID so unlisten works
        if (cmd === "plugin:event|listen") {
          return Promise.resolve(Math.floor(Math.random() * 100000));
        }
        // plugin:updater|check — return null (no update available)
        if (cmd === "plugin:updater|check") {
          return Promise.resolve(null);
        }
        // plugin:dialog|open — return null (user cancelled)
        if (cmd === "plugin:dialog|open") {
          return Promise.resolve(null);
        }
        // plugin:dialog|message — resolve immediately
        if (cmd === "plugin:dialog|message") {
          return Promise.resolve();
        }
        // Default for any other plugin command
        return Promise.resolve(null);
      }

      // Handle custom app commands
      if (cmd in responses) {
        return Promise.resolve(responses[cmd]);
      }

      console.warn(`[Tauri Mock] Unhandled command: ${cmd}`);
      return Promise.resolve(null);
    }

    // ── Wire up __TAURI_INTERNALS__ ──
    (window as any).__TAURI_INTERNALS__ = {
      invoke,
      transformCallback,
      unregisterCallback,
      convertFileSrc: (path: string) => path,
      metadata: {
        currentWindow: { label: "main" },
        currentWebview: { label: "main" },
      },
    };

    // ── Wire up __TAURI_EVENT_PLUGIN_INTERNALS__ ──
    // The event plugin uses a separate global for listener bookkeeping.
    // _unlisten() calls unregisterListener() before invoking plugin:event|unlisten.
    (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      unregisterListener: (_event: string, _eventId: number) => {
        // no-op in mock — just prevents the "Cannot read properties of undefined" error
      },
    };
  }, JSON.stringify(overrides || {}));
}
