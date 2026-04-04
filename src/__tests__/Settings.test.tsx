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

  it("renders provider sections (Claude, OpenAI, ElevenLabs)", async () => {
    render(<SettingsPanel onClose={onClose} />);

    expect(screen.getByText("Anthropic (Claude)")).toBeInTheDocument();
    expect(screen.getByText("OpenAI")).toBeInTheDocument();
    expect(screen.getByText("ElevenLabs TTS")).toBeInTheDocument();
  });

  it("shows Configured/Not set badges based on mocked status", async () => {
    render(<SettingsPanel onClose={onClose} />);

    // Wait for the async getProviderStatus to resolve
    // Claude has_key: true -> "Configured", OpenAI has_key: false -> "Not set"
    await waitFor(() => {
      const badges = screen.getAllByText("Configured");
      expect(badges.length).toBeGreaterThanOrEqual(1);
    });

    await waitFor(() => {
      const notSetBadges = screen.getAllByText("Not set");
      // OpenAI + possibly ElevenLabs (depends on mock) have "Not set"
      expect(notSetBadges.length).toBeGreaterThanOrEqual(1);
    });
  });

  it("shows Configured badge for ElevenLabs when key exists", async () => {
    render(<SettingsPanel onClose={onClose} />);

    // Our mock for get_elevenlabs_config returns a config with api_key
    await waitFor(() => {
      const badges = screen.getAllByText("Configured");
      // Claude + ElevenLabs should both show "Configured"
      expect(badges.length).toBe(2);
    });
  });

  it("typing in API key input and clicking Save calls set_api_key", async () => {
    const setApiKeyCalls: Array<{ provider: string; key: string }> = [];

    // Override mock to track set_api_key calls
    clearMocks();
    mockIPC((cmd, payload) => {
      const p = payload as Record<string, unknown> | undefined;
      switch (cmd) {
        case "get_provider_status":
          return [
            { provider: "claude", has_key: false, models: ["claude-sonnet-4-20250514"] },
            { provider: "openai", has_key: false, models: ["gpt-4o"] },
          ];
        case "get_elevenlabs_config":
          return null;
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

    // Wait for statuses to load
    await waitFor(() => {
      expect(screen.getAllByText("Not set").length).toBeGreaterThanOrEqual(2);
    });

    // Find the first password input (Claude section) and type a key
    const inputs = screen.getAllByPlaceholderText("Enter API key...");
    expect(inputs.length).toBeGreaterThanOrEqual(1);

    await user.type(inputs[0], "sk-test-key-12345");

    // Find and click the first Save button
    const saveButtons = screen.getAllByText("Save");
    await user.click(saveButtons[0]);

    // Wait for the validation + save flow
    await waitFor(() => {
      expect(setApiKeyCalls.length).toBe(1);
      expect(setApiKeyCalls[0].provider).toBe("claude");
      expect(setApiKeyCalls[0].key).toBe("sk-test-key-12345");
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

  it("renders the API Keys heading", () => {
    render(<SettingsPanel onClose={onClose} />);
    expect(screen.getByText("API Keys")).toBeInTheDocument();
  });
});
