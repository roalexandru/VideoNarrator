import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { clearMocks } from "@tauri-apps/api/mocks";
import { setupDefaultMocks, resetAllStores } from "./setup";
import { ConfigurationScreen } from "../features/configuration/ConfigurationScreen";
import { useConfigStore } from "../stores/configStore";

describe("ConfigurationScreen", () => {
  beforeEach(() => {
    resetAllStores();
    setupDefaultMocks();
  });

  afterEach(() => {
    clearMocks();
  });

  it("renders style cards including Product Demo", () => {
    render(<ConfigurationScreen />);
    expect(screen.getByText("Product Demo")).toBeInTheDocument();
    expect(screen.getByText("Executive Overview")).toBeInTheDocument();
    expect(screen.getByText("Technical Deep-Dive")).toBeInTheDocument();
    expect(screen.getByText("Teaser / Trailer")).toBeInTheDocument();
    expect(screen.getByText("Training Walkthrough")).toBeInTheDocument();
    expect(screen.getByText("Bug Review / Critique")).toBeInTheDocument();
  });

  it("clicking a style card updates the config store", () => {
    render(<ConfigurationScreen />);

    expect(useConfigStore.getState().style).toBe("product_demo");

    fireEvent.click(screen.getByText("Technical Deep-Dive"));
    expect(useConfigStore.getState().style).toBe("technical");

    fireEvent.click(screen.getByText("Executive Overview"));
    expect(useConfigStore.getState().style).toBe("executive");
  });

  it("language buttons render and clicking toggles language", () => {
    render(<ConfigurationScreen />);

    expect(screen.getByText(/English/)).toBeInTheDocument();
    expect(screen.getByText(/Japanese/)).toBeInTheDocument();
    expect(screen.getByText(/German/)).toBeInTheDocument();
    expect(screen.getByText(/French/)).toBeInTheDocument();

    expect(useConfigStore.getState().languages).toEqual(["en"]);

    fireEvent.click(screen.getByText(/Japanese/));
    expect(useConfigStore.getState().languages).toContain("ja");

    fireEvent.click(screen.getByText(/Japanese/));
    expect(useConfigStore.getState().languages).not.toContain("ja");
  });

  it("renders AI summary card with current provider", async () => {
    render(<ConfigurationScreen />);

    await waitFor(() => {
      expect(screen.getByText(/Anthropic \(Claude\)/)).toBeInTheDocument();
    });
  });

  it("renders Voice summary card", async () => {
    render(<ConfigurationScreen />);

    await waitFor(() => {
      expect(screen.getByText(/ElevenLabs/)).toBeInTheDocument();
    });
  });

  it("renders Configure buttons for AI and Voice", async () => {
    render(<ConfigurationScreen />);

    await waitFor(() => {
      const configureButtons = screen.getAllByText("Configure");
      expect(configureButtons.length).toBeGreaterThanOrEqual(2);
    });
  });

  it("shows Custom Prompt, Max Frames, and Temperature inline (no Project Overrides toggle)", () => {
    render(<ConfigurationScreen />);

    // All three controls live inline next to their related section, not
    // hidden behind an accordion.
    expect(screen.queryByText(/Project Overrides/)).not.toBeInTheDocument();
    expect(screen.getByText(/Custom Prompt/)).toBeInTheDocument();
    expect(screen.getByText(/Max Frames/)).toBeInTheDocument();
    expect(screen.getByText(/Temperature/)).toBeInTheDocument();
  });

  it("frame density buttons render and update store", () => {
    render(<ConfigurationScreen />);

    expect(screen.getByText("Light")).toBeInTheDocument();
    expect(screen.getByText("Medium")).toBeInTheDocument();
    expect(screen.getByText("Heavy")).toBeInTheDocument();

    expect(useConfigStore.getState().frameDensity).toBe("medium");

    fireEvent.click(screen.getByText("Heavy"));
    expect(useConfigStore.getState().frameDensity).toBe("heavy");

    fireEvent.click(screen.getByText("Light"));
    expect(useConfigStore.getState().frameDensity).toBe("light");
  });

  it("density buttons show descriptive subtitles", () => {
    render(<ConfigurationScreen />);

    expect(screen.getByText("Key points only")).toBeInTheDocument();
    expect(screen.getByText("Balanced coverage")).toBeInTheDocument();
    expect(screen.getByText("Detailed commentary")).toBeInTheDocument();
  });
});
