import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { clearMocks } from "@tauri-apps/api/mocks";
import { setupDefaultMocks, resetAllStores } from "./setup";
import { WizardLayout } from "../components/layout/WizardLayout";
import { useWizardStore, STEP_LABELS } from "../hooks/useWizardNavigation";

describe("WizardLayout", () => {
  beforeEach(() => {
    resetAllStores();
    useWizardStore.getState().reset();
    setupDefaultMocks();
  });

  afterEach(() => {
    clearMocks();
  });

  it("renders all step labels", () => {
    render(
      <WizardLayout>
        <div>Content</div>
      </WizardLayout>
    );

    for (const label of STEP_LABELS) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
  });

  it("renders children in the main content area", () => {
    render(
      <WizardLayout>
        <div>Test Content Area</div>
      </WizardLayout>
    );

    expect(screen.getByText("Test Content Area")).toBeInTheDocument();
  });

  it("active step has aria-current='step'", () => {
    // Default step is 0 (Project Setup)
    render(
      <WizardLayout>
        <div>Content</div>
      </WizardLayout>
    );

    const activeButton = screen.getByRole("button", { name: /Project Setup/i });
    expect(activeButton).toHaveAttribute("aria-current", "step");

    // Non-active steps should not have aria-current
    const editButton = screen.getByRole("button", { name: /Edit Video/i });
    expect(editButton).not.toHaveAttribute("aria-current");
  });

  it("step click handler navigates to the clicked step", async () => {
    const user = userEvent.setup();

    render(
      <WizardLayout>
        <div>Content</div>
      </WizardLayout>
    );

    // Click on Configuration (step 2)
    await user.click(screen.getByText("Configuration"));
    expect(useWizardStore.getState().currentStep).toBe(2);

    // Verify the clicked step now has aria-current
    const configButton = screen.getByRole("button", { name: /Configuration/i });
    expect(configButton).toHaveAttribute("aria-current", "step");
  });

  it("clicking different steps updates the active step", async () => {
    const user = userEvent.setup();

    render(
      <WizardLayout>
        <div>Content</div>
      </WizardLayout>
    );

    // Navigate to Export (step 5)
    await user.click(screen.getByText("Export"));
    expect(useWizardStore.getState().currentStep).toBe(5);

    // Navigate to Processing (step 3)
    await user.click(screen.getByText("Processing"));
    expect(useWizardStore.getState().currentStep).toBe(3);
  });

  it("renders Settings button when onOpenSettings is provided", () => {
    const onOpenSettings = vi.fn();

    render(
      <WizardLayout onOpenSettings={onOpenSettings}>
        <div>Content</div>
      </WizardLayout>
    );

    expect(screen.getByText("Settings")).toBeInTheDocument();
  });

  it("does not render Settings button when onOpenSettings is not provided", () => {
    render(
      <WizardLayout>
        <div>Content</div>
      </WizardLayout>
    );

    // The step labels include a gear icon but the "Settings" text button is only shown when onOpenSettings is provided
    const settingsButtons = screen.queryAllByText("Settings");
    expect(settingsButtons).toHaveLength(0);
  });

  it("renders Projects back button when onBackToLibrary is provided", () => {
    const onBackToLibrary = vi.fn();

    render(
      <WizardLayout onBackToLibrary={onBackToLibrary}>
        <div>Content</div>
      </WizardLayout>
    );

    expect(screen.getByText("Projects")).toBeInTheDocument();
  });

  it("renders version number", () => {
    render(
      <WizardLayout>
        <div>Content</div>
      </WizardLayout>
    );

    // __APP_VERSION__ is defined as "0.0.0-test" in vitest.config.ts
    expect(screen.getByText("v0.0.0-test")).toBeInTheDocument();
  });
});
