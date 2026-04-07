import { test, expect } from "@playwright/test";
import { mockTauriIPC } from "./tauri-mock";

test.describe("Smoke Tests", () => {
  test.beforeEach(async ({ page }) => {
    await mockTauriIPC(page);
  });

  test("app loads and shows title", async ({ page }) => {
    await page.goto("/");
    // The app should load without crashing
    await expect(page.locator("body")).toBeVisible();
    // Should see "Narrator" somewhere in the app
    await expect(page.getByText("Narrator")).toBeVisible({ timeout: 10000 });
  });
});
