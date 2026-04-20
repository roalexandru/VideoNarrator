import { test, expect } from "@playwright/test";
import { mockTauriIPC } from "./tauri-mock";

test.describe("Wizard Navigation", () => {
  test.beforeEach(async ({ page }) => {
    await mockTauriIPC(page);
  });

  test("app starts at the project library view", async ({ page }) => {
    await page.goto("/");
    // The app starts in library view — should see the "Projects" heading
    await expect(page.getByRole("heading", { name: "Projects" })).toBeVisible({ timeout: 10000 });
  });

  test("clicking New Project enters editor at step 0 (Project Setup)", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByText("New Project")).toBeVisible({ timeout: 10000 });
    await page.getByText("New Project").click();
    // Should now be in the wizard with Project Setup visible
    await expect(page.getByRole("heading", { name: "Project Setup" })).toBeVisible({ timeout: 5000 });
  });

  test("step labels are visible in the sidebar", async ({ page }) => {
    await page.goto("/");
    await page.getByText("New Project").click();
    await expect(page.getByRole("heading", { name: "Project Setup" })).toBeVisible({ timeout: 5000 });

    // All step labels in the sidebar nav
    const stepLabels = ["Project Setup", "Edit Video", "Configuration", "Processing", "Review", "Export"];
    for (const label of stepLabels) {
      // Use sidebar nav buttons with aria-current attribute (only step buttons have it)
      await expect(page.locator("aside button", { hasText: label }).first()).toBeVisible();
    }
  });

  test("active step is visually indicated with aria-current", async ({ page }) => {
    await page.goto("/");
    await page.getByText("New Project").click();
    await expect(page.getByRole("heading", { name: "Project Setup" })).toBeVisible({ timeout: 5000 });

    // The first step button should have aria-current="step"
    const projectSetupBtn = page.locator('aside button[aria-current="step"]');
    await expect(projectSetupBtn).toBeVisible();
    await expect(projectSetupBtn).toContainText("Project Setup");

    // Other step buttons in the sidebar should NOT have aria-current="step"
    const configBtn = page.locator("aside button", { hasText: "Configuration" });
    await expect(configBtn).not.toHaveAttribute("aria-current", "step");
  });

  test("can navigate to different steps by clicking sidebar buttons", async ({ page }) => {
    await page.goto("/");
    await page.getByText("New Project").click();
    await expect(page.getByRole("heading", { name: "Project Setup" })).toBeVisible({ timeout: 5000 });

    // Navigate to Configuration (step 2)
    await page.locator("aside button", { hasText: "Configuration" }).click();
    await expect(page.getByRole("heading", { name: "Configuration" })).toBeVisible({ timeout: 5000 });
    await expect(page.locator('aside button[aria-current="step"]')).toContainText("Configuration");

    // Navigate to Export (step 5) — use the sidebar button specifically
    await page.locator("aside button", { hasText: "Export" }).click();
    await expect(page.getByRole("heading", { name: "Export" })).toBeVisible({ timeout: 5000 });
    await expect(page.locator('aside button[aria-current="step"]')).toContainText("Export");

    // Navigate back to Project Setup (step 0)
    await page.locator("aside button", { hasText: "Project Setup" }).click();
    await expect(page.getByRole("heading", { name: "Project Setup" })).toBeVisible({ timeout: 5000 });
  });

  test("Projects button in sidebar navigates back to project library", async ({ page }) => {
    await page.goto("/");
    await page.getByText("New Project").click();
    await expect(page.getByRole("heading", { name: "Project Setup" })).toBeVisible({ timeout: 5000 });

    // Click the "Projects" back button in the sidebar — navigates directly to library
    await page.locator("aside button", { hasText: "Projects" }).click();

    // Should be back at the library
    await expect(page.getByRole("heading", { name: "Projects", exact: true })).toBeVisible({ timeout: 5000 });
  });

  test("Settings button is visible in the sidebar", async ({ page }) => {
    await page.goto("/");
    await page.getByText("New Project").click();
    await expect(page.getByRole("heading", { name: "Project Setup" })).toBeVisible({ timeout: 5000 });

    await expect(page.locator("aside button", { hasText: "Settings" })).toBeVisible();
  });
});
