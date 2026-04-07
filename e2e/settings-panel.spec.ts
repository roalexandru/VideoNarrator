import { test, expect } from "@playwright/test";
import { mockTauriIPC } from "./tauri-mock";

test.describe("Settings Panel", () => {
  test.describe("from editor view", () => {
    test.beforeEach(async ({ page }) => {
      await mockTauriIPC(page);
      await page.goto("/");
      await page.getByText("New Project").click();
      await expect(page.getByRole("heading", { name: "Project Setup" })).toBeVisible({ timeout: 10000 });
    });

    test("settings button opens the settings panel", async ({ page }) => {
      await page.locator("aside button", { hasText: "Settings" }).click();
      // Settings panel should appear — the modal has "Settings" heading
      await expect(page.getByText("Settings", { exact: true }).last()).toBeVisible({ timeout: 5000 });
      // And should show tab buttons
      await expect(page.getByRole("button", { name: "Providers", exact: true })).toBeVisible();
    });

    test("settings panel has provider tabs", async ({ page }) => {
      await page.locator("aside button", { hasText: "Settings" }).click();
      await expect(page.getByRole("button", { name: "Providers", exact: true })).toBeVisible({ timeout: 5000 });
      // The three tabs in the settings panel
      await expect(page.getByRole("button", { name: "AI", exact: true })).toBeVisible();
      await expect(page.getByRole("button", { name: "Voice", exact: true })).toBeVisible();
    });

    test("settings panel can be closed with Done button", async ({ page }) => {
      await page.locator("aside button", { hasText: "Settings" }).click();
      await expect(page.getByRole("button", { name: "Providers", exact: true })).toBeVisible({ timeout: 5000 });

      // Click the Done button to close
      await page.getByRole("button", { name: "Done" }).click();

      // The settings panel should be gone — the Providers tab button should not be visible
      await expect(page.getByRole("button", { name: "Providers", exact: true })).not.toBeVisible({ timeout: 3000 });
    });

    test("providers tab shows AI provider names", async ({ page }) => {
      await page.locator("aside button", { hasText: "Settings" }).click();
      await expect(page.getByRole("button", { name: "Providers", exact: true })).toBeVisible({ timeout: 5000 });

      // Click on the Providers tab to make sure we're on it
      await page.getByRole("button", { name: "Providers", exact: true }).click();

      // Should see provider names from the mock — "Anthropic (Claude)" is a provider
      await expect(page.getByText("Anthropic (Claude)")).toBeVisible({ timeout: 5000 });
      await expect(page.getByText("OpenAI")).toBeVisible();
    });

    test("can switch to AI tab", async ({ page }) => {
      await page.locator("aside button", { hasText: "Settings" }).click();
      await expect(page.getByRole("button", { name: "AI", exact: true })).toBeVisible({ timeout: 5000 });

      await page.getByRole("button", { name: "AI", exact: true }).click();
      // Should see AI-related content (provider and model selection)
      await expect(page.getByText("AI Provider")).toBeVisible({ timeout: 5000 });
    });

    test("can switch to Voice tab", async ({ page }) => {
      await page.locator("aside button", { hasText: "Settings" }).click();
      await expect(page.getByRole("button", { name: "Voice", exact: true })).toBeVisible({ timeout: 5000 });

      await page.getByRole("button", { name: "Voice", exact: true }).click();
      // Should see voice-related content — ElevenLabs is the first TTS provider
      await expect(page.getByText("ElevenLabs", { exact: true }).first()).toBeVisible({ timeout: 5000 });
    });
  });

  test.describe("from library view", () => {
    test.beforeEach(async ({ page }) => {
      await mockTauriIPC(page);
      await page.goto("/");
      await expect(page.getByRole("heading", { name: "Projects" })).toBeVisible({ timeout: 10000 });
    });

    test("settings gear icon in library opens settings panel", async ({ page }) => {
      // The library view has a small gear icon button in the top-right
      // It's the only button in the top bar area (28x28 pixels with SVG)
      const settingsBtn = page.locator("button").filter({
        has: page.locator('svg path[d*="M19.4 15"]'),
      }).first();
      await settingsBtn.click();

      // Should see the settings panel with Providers tab
      await expect(page.getByRole("button", { name: "Providers", exact: true })).toBeVisible({ timeout: 5000 });
    });
  });
});
