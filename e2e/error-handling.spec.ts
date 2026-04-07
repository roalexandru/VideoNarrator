import { test, expect } from "@playwright/test";
import { mockTauriIPC } from "./tauri-mock";

test.describe("Error Handling", () => {
  test("app handles missing ffmpeg gracefully", async ({ page }) => {
    // Override check_ffmpeg to simulate failure
    await mockTauriIPC(page, {
      check_ffmpeg: "__THROW__:ffmpeg not found in PATH",
    });

    // We need to make the mock throw instead of resolve
    // The current mock resolves with the value — let's use a special init script
    await page.addInitScript(() => {
      const orig = (window as any).__TAURI_INTERNALS__.invoke;
      (window as any).__TAURI_INTERNALS__.invoke = (cmd: string, args?: any) => {
        if (cmd === "check_ffmpeg") {
          return Promise.reject("ffmpeg not found in PATH");
        }
        return orig(cmd, args);
      };
    });

    await page.goto("/");
    // App should still load — it should handle the ffmpeg error gracefully
    await expect(page.locator("body")).toBeVisible();
    // The project library should still be accessible
    await expect(page.getByRole("heading", { name: "Projects" })).toBeVisible({ timeout: 10000 });
  });

  test("app handles failed provider status API call", async ({ page }) => {
    await mockTauriIPC(page, {});

    // Override to make get_provider_status throw
    await page.addInitScript(() => {
      const orig = (window as any).__TAURI_INTERNALS__.invoke;
      (window as any).__TAURI_INTERNALS__.invoke = (cmd: string, args?: any) => {
        if (cmd === "get_provider_status") {
          return Promise.reject("API error: unable to fetch provider status");
        }
        return orig(cmd, args);
      };
    });

    await page.goto("/");
    // App should still load despite the API error
    await expect(page.getByRole("heading", { name: "Projects" })).toBeVisible({ timeout: 10000 });

    // Should still be able to create a new project
    await page.getByText("New Project").click();
    await expect(page.getByRole("heading", { name: "Project Setup" })).toBeVisible({ timeout: 5000 });
  });

  test("app handles failed project listing", async ({ page }) => {
    await mockTauriIPC(page, {});

    await page.addInitScript(() => {
      const orig = (window as any).__TAURI_INTERNALS__.invoke;
      (window as any).__TAURI_INTERNALS__.invoke = (cmd: string, args?: any) => {
        if (cmd === "list_projects") {
          return Promise.reject("Failed to list projects");
        }
        return orig(cmd, args);
      };
    });

    await page.goto("/");
    // App should show the library view — projects may just be empty
    await expect(page.locator("body")).toBeVisible();
    // The heading should still appear
    await expect(page.getByRole("heading", { name: "Projects", exact: true })).toBeVisible({ timeout: 10000 });
  });

  test("app loads even when telemetry check fails", async ({ page }) => {
    await mockTauriIPC(page, {});

    await page.addInitScript(() => {
      const orig = (window as any).__TAURI_INTERNALS__.invoke;
      (window as any).__TAURI_INTERNALS__.invoke = (cmd: string, args?: any) => {
        if (cmd === "get_telemetry_enabled") {
          return Promise.reject("Telemetry check failed");
        }
        return orig(cmd, args);
      };
    });

    await page.goto("/");
    // App should still load
    await expect(page.getByRole("heading", { name: "Projects" })).toBeVisible({ timeout: 10000 });
  });

  test("app handles ElevenLabs config failure gracefully", async ({ page }) => {
    await mockTauriIPC(page, {});

    await page.addInitScript(() => {
      const orig = (window as any).__TAURI_INTERNALS__.invoke;
      (window as any).__TAURI_INTERNALS__.invoke = (cmd: string, args?: any) => {
        if (cmd === "get_elevenlabs_config") {
          return Promise.reject("ElevenLabs config not found");
        }
        return orig(cmd, args);
      };
    });

    await page.goto("/");
    await page.getByText("New Project").click();
    await expect(page.getByRole("heading", { name: "Project Setup" })).toBeVisible({ timeout: 10000 });

    // Navigate to Configuration — it should still render despite ElevenLabs failure
    await page.getByRole("button", { name: "Configuration" }).click();
    await expect(page.getByRole("heading", { name: "Configuration" })).toBeVisible({ timeout: 5000 });
  });
});
