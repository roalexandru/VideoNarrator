import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";
import { resetAllStores, setupDefaultMocks } from "./setup";
import { SettingsPanel } from "../features/settings/SettingsPanel";

describe("SettingsPanel", () => {
  const onClose = vi.fn();

  beforeEach(() => {
    resetAllStores();
    setupDefaultMocks();
    onClose.mockClear();
  });

  afterEach(() => {
    clearMocks();
  });

  it("renders the Settings heading", () => {
    render(<SettingsPanel onClose={onClose} />);
    expect(screen.getByText("Settings")).toBeInTheDocument();
  });

  it("renders tab navigation with Providers, AI, Voice", () => {
    render(<SettingsPanel onClose={onClose} />);
    expect(screen.getByText("Providers")).toBeInTheDocument();
    expect(screen.getByText("AI")).toBeInTheDocument();
    expect(screen.getByText("Voice")).toBeInTheDocument();
  });

  it("renders all AI provider names on Providers tab", async () => {
    render(<SettingsPanel onClose={onClose} />);

    expect(screen.getByText("Anthropic (Claude)")).toBeInTheDocument();
    expect(screen.getByText("OpenAI")).toBeInTheDocument();
    expect(screen.getByText("Google (Gemini)")).toBeInTheDocument();
  });

  it("renders TTS provider names on Providers tab", async () => {
    render(<SettingsPanel onClose={onClose} />);

    expect(screen.getByText("ElevenLabs")).toBeInTheDocument();
    expect(screen.getByText("Azure TTS")).toBeInTheDocument();
  });

  it("renders preferences section with telemetry on Providers tab", async () => {
    render(<SettingsPanel onClose={onClose} />);

    await waitFor(() => {
      expect(screen.getByText("Anonymous Usage Analytics")).toBeInTheDocument();
    });
  });

  it("renders Done button that calls onClose", async () => {
    const user = userEvent.setup();
    render(<SettingsPanel onClose={onClose} />);

    const doneButton = screen.getByText("Done");
    expect(doneButton).toBeInTheDocument();

    await user.click(doneButton);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("switches to AI tab when clicked", async () => {
    const user = userEvent.setup();
    render(<SettingsPanel onClose={onClose} />);

    await user.click(screen.getByText("AI"));

    await waitFor(() => {
      expect(screen.getByText(/AI Provider/i)).toBeInTheDocument();
    });
  });

  it("opens to initialTab when provided", async () => {
    render(<SettingsPanel onClose={onClose} initialTab="voice" />);

    await waitFor(() => {
      expect(screen.getByText(/TTS Provider/i)).toBeInTheDocument();
    });
  });

  it("typing in API key input and clicking Save calls set_api_key", async () => {
    const setApiKeyCalls: Array<{ provider: string; key: string }> = [];

    clearMocks();
    mockIPC((cmd, payload) => {
      const p = payload as Record<string, unknown> | undefined;
      switch (cmd) {
        case "get_provider_status":
          return [
            { provider: "claude", has_key: false, models: ["claude-sonnet-4-20250514"] },
            { provider: "openai", has_key: false, models: ["gpt-4o"] },
            { provider: "gemini", has_key: false, models: ["gemini-2.5-flash"] },
          ];
        case "get_elevenlabs_config":
          return null;
        case "get_azure_tts_config":
          return null;
        case "get_telemetry_enabled":
          return true;
        case "validate_api_key_cmd":
          return true;
        case "set_api_key":
          setApiKeyCalls.push({
            provider: p?.provider as string,
            key: p?.key as string,
          });
          return null;
        default:
          return null;
      }
    });

    const user = userEvent.setup();
    render(<SettingsPanel onClose={onClose} />);

    // Wait for provider rows to render
    await waitFor(() => {
      expect(screen.getByText("Anthropic (Claude)")).toBeInTheDocument();
    });

    const inputs = screen.getAllByPlaceholderText("Enter API key...");
    expect(inputs.length).toBeGreaterThanOrEqual(1);

    await user.type(inputs[0], "sk-test-key-12345");

    const saveButtons = screen.getAllByText("Save");
    await user.click(saveButtons[0]);

    await waitFor(() => {
      expect(setApiKeyCalls.length).toBe(1);
      expect(setApiKeyCalls[0].provider).toBe("claude");
      expect(setApiKeyCalls[0].key).toBe("sk-test-key-12345");
    });
  });
});
