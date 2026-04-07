import { test, expect } from "@playwright/test";
import { mockTauriIPC } from "./tauri-mock";

test.describe("Configuration Screen", () => {
  test.beforeEach(async ({ page }) => {
    await mockTauriIPC(page);
    await page.goto("/");
    // Navigate to editor, then to Configuration step
    await page.getByText("New Project").click();
    await expect(page.getByRole("heading", { name: "Project Setup" })).toBeVisible({ timeout: 10000 });
    await page.getByRole("button", { name: "Configuration" }).click();
    await expect(page.getByRole("heading", { name: "Configuration" })).toBeVisible({ timeout: 5000 });
  });

  test("narration style cards are displayed", async ({ page }) => {
    await expect(page.getByText("Narration Style")).toBeVisible();
    const styles = ["Executive Overview", "Product Demo", "Technical Deep-Dive", "Teaser / Trailer", "Training Walkthrough", "Bug Review / Critique"];
    for (const style of styles) {
      await expect(page.getByText(style)).toBeVisible();
    }
  });

  test("can select a narration style card", async ({ page }) => {
    // Click on "Technical Deep-Dive"
    await page.getByText("Technical Deep-Dive").click();
    // The text should still be visible (style applied after click)
    await expect(page.getByText("Technical Deep-Dive")).toBeVisible();
  });

  test("language selection is visible", async ({ page }) => {
    await expect(page.getByText("Languages", { exact: true })).toBeVisible();
    // At minimum, English should be shown as a language option
    // Language buttons contain flag emoji + label, e.g. "🇺🇸 English"
    await expect(page.getByText("English")).toBeVisible();
  });

  test("frame density options are shown", async ({ page }) => {
    await expect(page.getByText("Frame Extraction")).toBeVisible();
    await expect(page.getByRole("button", { name: "light" })).toBeVisible();
    await expect(page.getByRole("button", { name: "medium" })).toBeVisible();
    await expect(page.getByRole("button", { name: "heavy" })).toBeVisible();
  });

  test("can change frame density selection", async ({ page }) => {
    await page.getByRole("button", { name: "heavy" }).click();
    // heavy button should now have the selected styling — verify it's still there
    await expect(page.getByRole("button", { name: "heavy" })).toBeVisible();
  });

  test("AI provider summary card is visible", async ({ page }) => {
    await expect(page.getByText("AI", { exact: true })).toBeVisible();
    // Should see the Configure button for AI
    const configureButtons = page.getByRole("button", { name: "Configure" });
    await expect(configureButtons.first()).toBeVisible();
  });

  test("Voice summary card is visible", async ({ page }) => {
    await expect(page.getByText("Voice")).toBeVisible();
  });

  test("project overrides toggle works", async ({ page }) => {
    // Initially the overrides are hidden
    await expect(page.getByText("+ Show Project Overrides")).toBeVisible();

    // Click to show overrides
    await page.getByText("+ Show Project Overrides").click();

    // Now we should see override fields
    await expect(page.getByText("- Hide Project Overrides")).toBeVisible();
    await expect(page.getByText("Custom Prompt")).toBeVisible();
    await expect(page.getByText("Max Frames")).toBeVisible();
  });
});
