import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";
import { resetAllStores, setupDefaultMocks } from "./setup";
import { SettingsPanel } from "../features/settings/SettingsPanel";
import { PrivacyPolicy } from "../features/legal/PrivacyPolicy";
import { TermsOfService } from "../features/legal/TermsOfService";

describe("Telemetry toggle in Settings", () => {
  const onClose = vi.fn();

  beforeEach(() => {
    resetAllStores();
    setupDefaultMocks();
    onClose.mockClear();
  });

  afterEach(() => {
    clearMocks();
  });

  it("renders the telemetry toggle section", async () => {
    render(<SettingsPanel onClose={onClose} initialTab="providers" />);

    expect(screen.getByText("Anonymous Usage Analytics")).toBeInTheDocument();
    expect(
      screen.getByText(/Help improve Narrator/)
    ).toBeInTheDocument();
  });

  it("toggle calls set_telemetry_enabled with false when clicked", async () => {
    const calls: Array<{ enabled: boolean }> = [];

    clearMocks();
    mockIPC((cmd, payload) => {
      const p = payload as Record<string, unknown> | undefined;
      switch (cmd) {
        case "get_provider_status":
          return [
            { provider: "claude", has_key: true, models: [] },
            { provider: "openai", has_key: false, models: [] },
            { provider: "gemini", has_key: false, models: [] },
          ];
        case "get_elevenlabs_config":
          return null;
        case "get_azure_tts_config":
          return null;
        case "get_telemetry_enabled":
          return true;
        case "set_telemetry_enabled":
          calls.push({ enabled: p?.enabled as boolean });
          return null;
        default:
          return null;
      }
    });

    const user = userEvent.setup();
    render(<SettingsPanel onClose={onClose} initialTab="providers" />);

    // Wait for telemetry state to load
    await waitFor(() => {
      expect(screen.getByText("Anonymous Usage Analytics")).toBeInTheDocument();
    });

    // The toggle button is inside the analytics row
    const analyticsSection = screen
      .getByText("Anonymous Usage Analytics")
      .closest("div")!
      .parentElement!;
    const toggleButton = analyticsSection.querySelector(
      'button[style*="border-radius"]'
    ) as HTMLElement;
    expect(toggleButton).toBeTruthy();

    await user.click(toggleButton);

    await waitFor(() => {
      expect(calls.length).toBe(1);
      expect(calls[0].enabled).toBe(false);
    });
  });

  it("renders legal links when callbacks are provided", () => {
    const onShowPrivacy = vi.fn();
    const onShowTerms = vi.fn();

    render(
      <SettingsPanel
        onClose={onClose}
        onShowPrivacyPolicy={onShowPrivacy}
        onShowTerms={onShowTerms}
        initialTab="providers"
      />
    );

    expect(screen.getByText("Privacy Policy")).toBeInTheDocument();
    expect(screen.getByText("Terms of Service")).toBeInTheDocument();
  });

  it("does not render legal links when callbacks are omitted", () => {
    render(<SettingsPanel onClose={onClose} initialTab="providers" />);

    expect(screen.queryByText("Privacy Policy")).not.toBeInTheDocument();
    expect(screen.queryByText("Terms of Service")).not.toBeInTheDocument();
  });
});

describe("PrivacyPolicy modal", () => {
  it("renders key sections of the privacy policy", () => {
    const onClose = vi.fn();
    render(<PrivacyPolicy onClose={onClose} />);

    expect(screen.getByText("Privacy Policy")).toBeInTheDocument();
    expect(screen.getByText("What We Collect")).toBeInTheDocument();
    expect(screen.getByText("What We Do NOT Collect")).toBeInTheDocument();
    expect(screen.getByText("How Data Is Processed")).toBeInTheDocument();
    expect(screen.getByText("Opt-Out")).toBeInTheDocument();
    expect(screen.getByText("Third-Party Services")).toBeInTheDocument();
    expect(screen.getByText("Local Data Storage")).toBeInTheDocument();
  });

  it("closes on Done button click", async () => {
    const onClose = vi.fn();
    const user = userEvent.setup();
    render(<PrivacyPolicy onClose={onClose} />);

    await user.click(screen.getByText("Done"));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("closes on Escape key", async () => {
    const onClose = vi.fn();
    const user = userEvent.setup();
    render(<PrivacyPolicy onClose={onClose} />);

    await user.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});

describe("TermsOfService modal", () => {
  it("renders key sections of the terms", () => {
    const onClose = vi.fn();
    render(<TermsOfService onClose={onClose} />);

    expect(screen.getByText("Terms of Service")).toBeInTheDocument();
    expect(screen.getByText("License")).toBeInTheDocument();
    expect(screen.getByText("Disclaimer of Warranty")).toBeInTheDocument();
    expect(screen.getByText("AI-Generated Content")).toBeInTheDocument();
    expect(screen.getByText("API Keys & Third-Party Services")).toBeInTheDocument();
    expect(screen.getByText("Data Ownership")).toBeInTheDocument();
  });

  it("closes on Done button click", async () => {
    const onClose = vi.fn();
    const user = userEvent.setup();
    render(<TermsOfService onClose={onClose} />);

    await user.click(screen.getByText("Done"));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("closes on Escape key", async () => {
    const onClose = vi.fn();
    const user = userEvent.setup();
    render(<TermsOfService onClose={onClose} />);

    await user.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
