import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
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

    // Default style is product_demo
    expect(useConfigStore.getState().style).toBe("product_demo");

    fireEvent.click(screen.getByText("Technical Deep-Dive"));
    expect(useConfigStore.getState().style).toBe("technical");

    fireEvent.click(screen.getByText("Executive Overview"));
    expect(useConfigStore.getState().style).toBe("executive");
  });

  it("language buttons render and clicking toggles language", () => {
    render(<ConfigurationScreen />);

    // English should be present
    expect(screen.getByText(/English/)).toBeInTheDocument();
    expect(screen.getByText(/Japanese/)).toBeInTheDocument();
    expect(screen.getByText(/German/)).toBeInTheDocument();
    expect(screen.getByText(/French/)).toBeInTheDocument();

    // Initially only "en" is in languages
    expect(useConfigStore.getState().languages).toEqual(["en"]);

    // Toggle Japanese on
    fireEvent.click(screen.getByText(/Japanese/));
    expect(useConfigStore.getState().languages).toContain("ja");

    // Toggle Japanese off
    fireEvent.click(screen.getByText(/Japanese/));
    expect(useConfigStore.getState().languages).not.toContain("ja");
  });

  it("AI provider cards render", () => {
    render(<ConfigurationScreen />);

    expect(screen.getByText("Anthropic (Claude)")).toBeInTheDocument();
    expect(screen.getByText("OpenAI")).toBeInTheDocument();
    expect(screen.getByText("Claude Sonnet 4")).toBeInTheDocument();
    expect(screen.getByText("GPT-4o")).toBeInTheDocument();
  });

  it("clicking an AI provider card updates the config store", () => {
    render(<ConfigurationScreen />);

    expect(useConfigStore.getState().aiProvider).toBe("claude");

    fireEvent.click(screen.getByText("OpenAI"));
    expect(useConfigStore.getState().aiProvider).toBe("openai");
    expect(useConfigStore.getState().model).toBe("gpt-4o");
  });

  it("advanced settings toggle shows and hides the panel", () => {
    render(<ConfigurationScreen />);

    // Advanced panel should be hidden by default
    expect(screen.queryByText(/Temperature/)).not.toBeInTheDocument();

    // Click to show
    fireEvent.click(screen.getByText("+ Show Advanced"));
    expect(screen.getByText(/Temperature/)).toBeInTheDocument();
    expect(screen.getByText(/Max Frames/)).toBeInTheDocument();
    expect(screen.getByText(/Custom Prompt/)).toBeInTheDocument();

    // Click to hide
    fireEvent.click(screen.getByText("- Hide Advanced"));
    expect(screen.queryByText(/Max Frames/)).not.toBeInTheDocument();
  });

  it("frame density buttons render and update store", () => {
    render(<ConfigurationScreen />);

    expect(screen.getByText("light")).toBeInTheDocument();
    expect(screen.getByText("medium")).toBeInTheDocument();
    expect(screen.getByText("heavy")).toBeInTheDocument();

    expect(useConfigStore.getState().frameDensity).toBe("medium");

    fireEvent.click(screen.getByText("heavy"));
    expect(useConfigStore.getState().frameDensity).toBe("heavy");

    fireEvent.click(screen.getByText("light"));
    expect(useConfigStore.getState().frameDensity).toBe("light");
  });
});
