import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  workers: 1,
  reporter: process.env.CI
    ? [["html", { open: "never" }], ["github"]]
    : [["html", { open: "never" }], ["list"]],
  timeout: 30000,
  outputDir: "test-results",
  use: {
    baseURL: "http://localhost:1420",
    // Capture full trace on every failure — includes DOM snapshots, network, console, actions
    trace: "retain-on-failure",
    // Screenshot on every failure — embedded in the HTML report
    screenshot: "only-on-failure",
    // Record video on failure — shows exact steps leading to the crash
    video: "retain-on-failure",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: {
    command: "pnpm dev",
    url: "http://localhost:1420",
    reuseExistingServer: !process.env.CI,
    timeout: 30000,
  },
});
