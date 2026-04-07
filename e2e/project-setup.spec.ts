import { test, expect } from "@playwright/test";
import { mockTauriIPC } from "./tauri-mock";

test.describe("Project Setup Screen", () => {
  test.beforeEach(async ({ page }) => {
    await mockTauriIPC(page);
    await page.goto("/");
    // Navigate to editor by clicking New Project
    await page.getByText("New Project").click();
    await expect(page.getByRole("heading", { name: "Project Setup" })).toBeVisible({ timeout: 10000 });
  });

  test("title input is visible and editable", async ({ page }) => {
    const titleInput = page.getByPlaceholder("e.g., UiPath Studio Walkthrough");
    await expect(titleInput).toBeVisible();
    await titleInput.fill("My Test Project");
    await expect(titleInput).toHaveValue("My Test Project");
  });

  test("description textarea is visible and editable", async ({ page }) => {
    const descInput = page.getByPlaceholder("What should the viewer learn?");
    await expect(descInput).toBeVisible();
    await descInput.fill("A description of the video");
    await expect(descInput).toHaveValue("A description of the video");
  });

  test("video file selection area is visible", async ({ page }) => {
    await expect(page.getByText("Select Video File")).toBeVisible();
    await expect(page.getByText("MP4, MOV, AVI, MKV, WebM")).toBeVisible();
  });

  test("record screen option is visible", async ({ page }) => {
    await expect(page.getByText("Record Screen")).toBeVisible();
    await expect(page.getByText("Capture your screen")).toBeVisible();
  });

  test("context documents section is visible", async ({ page }) => {
    // The section label is rendered in uppercase via CSS text-transform
    await expect(page.getByText("Context Documents", { exact: true })).toBeVisible();
  });

  test("project details section has title label marked required", async ({ page }) => {
    await expect(page.getByText("Title *")).toBeVisible();
    await expect(page.getByText("Description", { exact: true })).toBeVisible();
  });

  test("title field shows validation error when blurred empty", async ({ page }) => {
    const titleInput = page.getByPlaceholder("e.g., UiPath Studio Walkthrough");
    await titleInput.click();
    await titleInput.blur();
    await expect(page.getByText("Title is required")).toBeVisible();
  });

  test("Video File section label is visible", async ({ page }) => {
    // The section label "Video File" is rendered in uppercase via CSS text-transform
    await expect(page.getByText("Video File", { exact: true })).toBeVisible();
  });

  test("add context documents button is visible", async ({ page }) => {
    await expect(page.getByRole("button", { name: "+ Add" })).toBeVisible();
  });

  test("helper text for context documents is shown when empty", async ({ page }) => {
    await expect(page.getByText("Brand guides, product docs, or glossaries improve narration quality.")).toBeVisible();
  });
});
