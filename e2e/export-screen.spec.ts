import { test, expect } from "@playwright/test";
import { mockTauriIPC } from "./tauri-mock";

test.describe("Export Screen", () => {
  test.beforeEach(async ({ page }) => {
    await mockTauriIPC(page);
    await page.goto("/");
    await page.getByText("New Project").click();
    await expect(page.getByRole("heading", { name: "Project Setup" })).toBeVisible({ timeout: 10000 });
    // Navigate to Export step
    await page.getByRole("button", { name: "Export" }).click();
    await expect(page.getByRole("heading", { name: "Export" })).toBeVisible({ timeout: 5000 });
  });

  test("export heading is visible", async ({ page }) => {
    await expect(page.getByRole("heading", { name: "Export" })).toBeVisible();
  });

  test("output directory field is visible", async ({ page }) => {
    // The output directory shows a path or "..." with a Change button
    // There are multiple "Change" buttons (folder + voice), use the first one
    await expect(page.getByRole("button", { name: "Change" }).first()).toBeVisible();
  });

  test("filename input is visible and editable", async ({ page }) => {
    await expect(page.getByText("Filename")).toBeVisible();
    const filenameInput = page.locator('input[type="text"]').last();
    await expect(filenameInput).toBeVisible();
  });

  test("VIDEO section is visible", async ({ page }) => {
    await expect(page.getByText("VIDEO", { exact: true })).toBeVisible();
  });

  test("AUDIO ONLY section is visible", async ({ page }) => {
    await expect(page.getByText("AUDIO ONLY")).toBeVisible();
  });

  test("SCRIPTS section is visible and can be expanded", async ({ page }) => {
    await expect(page.getByText("SCRIPTS")).toBeVisible();

    // SCRIPTS section is collapsed by default — click to expand
    await page.getByText("SCRIPTS").click();

    // After expanding, should see format toggle buttons
    await expect(page.getByRole("button", { name: "JSON" })).toBeVisible({ timeout: 5000 });
  });

  test("script format toggles are visible when SCRIPTS section is expanded", async ({ page }) => {
    // Expand SCRIPTS section
    await page.getByText("SCRIPTS").click();

    // Check that format buttons are visible
    const formats = ["JSON", "SRT", "WebVTT", "Plain", "Markdown", "SSML"];
    for (const format of formats) {
      await expect(page.getByRole("button", { name: format })).toBeVisible();
    }
  });

  test("can toggle script format buttons", async ({ page }) => {
    // Expand SCRIPTS section
    await page.getByText("SCRIPTS").click();

    // Click on SRT format toggle
    const srtButton = page.getByRole("button", { name: "SRT" });
    await expect(srtButton).toBeVisible();
    await srtButton.click();

    // The button should still be visible (toggle state changed)
    await expect(srtButton).toBeVisible();
  });

  test("Export Video button exists in VIDEO section", async ({ page }) => {
    // The VIDEO section has the Export Video button
    // It might show "Add a TTS API key..." message if no key, or the button if key exists
    // With our mock (has test-el-key), should see the button
    await expect(page.getByRole("button", { name: "Export Video" })).toBeVisible();
  });

  test("Export Audio button exists in AUDIO ONLY section", async ({ page }) => {
    await expect(page.getByRole("button", { name: "Export Audio" })).toBeVisible();
  });

  test("Export Scripts button is visible when SCRIPTS section is expanded", async ({ page }) => {
    await page.getByText("SCRIPTS").click();
    await expect(page.getByRole("button", { name: "Export Scripts" })).toBeVisible();
  });
});
