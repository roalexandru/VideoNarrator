import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, waitFor, act } from "@testing-library/react";
import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";
import { resetAllStores, setupDefaultMocks } from "./setup";
import App from "../App";
import { useProjectStore } from "../stores/projectStore";
import { useConfigStore } from "../stores/configStore";
import { useWizardStore } from "../hooks/useWizardNavigation";

let menuEventHandler: ((event: { payload: string }) => void) | null = null;

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((eventName: string, handler: (event: { payload: string }) => void) => {
    if (eventName === "menu-event") menuEventHandler = handler;
    return Promise.resolve(() => {});
  }),
}));

const mockWindow = {
  minimize: vi.fn(), close: vi.fn(), setFocus: vi.fn(),
  startDragging: vi.fn(), unminimize: vi.fn(),
  isFullscreen: vi.fn().mockResolvedValue(false),
  setFullscreen: vi.fn(),
};

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => mockWindow,
}));

async function emitMenuEvent(action: string) {
  expect(menuEventHandler).not.toBeNull();
  await act(async () => { await menuEventHandler!({ payload: action }); });
}

describe("Menu events", () => {
  beforeEach(() => { resetAllStores(); setupDefaultMocks(); menuEventHandler = null; });
  afterEach(() => { clearMocks(); });

  it("registers a menu-event listener on mount", () => {
    render(<App />);
    expect(menuEventHandler).not.toBeNull();
  });

  it("syncs menu context on mount (library = no project)", () => {
    const contextCalls: unknown[] = [];
    clearMocks();
    mockIPC((cmd, payload) => {
      if (cmd === "set_menu_context") { contextCalls.push(payload); return null; }
      if (cmd === "list_projects") return [];
      if (cmd === "list_styles") return [];
      if (cmd === "get_provider_status") return [];
      return null;
    });
    render(<App />);
    expect(contextCalls.length).toBeGreaterThanOrEqual(1);
    expect((contextCalls[0] as Record<string, unknown>).hasProject).toBe(false);
  });

  it("new_project from library goes straight to editor", async () => {
    render(<App />);
    await emitMenuEvent("new_project");
    expect(useProjectStore.getState().projectId).not.toBe("");
    expect(useWizardStore.getState().currentStep).toBe(0);
  });

  it("new_project from editor shows save dialog", async () => {
    render(<App />);
    await emitMenuEvent("new_project");
    const firstId = useProjectStore.getState().projectId;

    await emitMenuEvent("new_project");

    expect(useProjectStore.getState().projectId).toBe(firstId);
    await waitFor(() => {
      expect(screen.getByText("Save before closing?")).toBeInTheDocument();
      expect(screen.getByText("Save & New")).toBeInTheDocument();
      expect(screen.getByText("Don't Save")).toBeInTheDocument();
      expect(screen.getByText("Cancel")).toBeInTheDocument();
    });
  });

  it("Don't Save discards and creates new project", async () => {
    render(<App />);
    await emitMenuEvent("new_project");
    useProjectStore.getState().setTitle("Important Work");

    await emitMenuEvent("new_project");
    await act(async () => { screen.getByText("Don't Save").click(); });

    expect(useProjectStore.getState().title).toBe("");
    expect(useProjectStore.getState().projectId).not.toBe("");
  });

  it("Cancel keeps current project untouched", async () => {
    render(<App />);
    await emitMenuEvent("new_project");
    useProjectStore.getState().setTitle("Important Work");
    const id = useProjectStore.getState().projectId;

    await emitMenuEvent("new_project");
    await waitFor(() => { expect(screen.getByText("Save before closing?")).toBeInTheDocument(); });

    await act(async () => { screen.getByText("Cancel").click(); });

    expect(useProjectStore.getState().projectId).toBe(id);
    expect(useProjectStore.getState().title).toBe("Important Work");
  });

  it("Save & New saves then creates new project", async () => {
    const saveCalls: unknown[] = [];
    clearMocks();
    mockIPC((cmd, payload) => {
      if (cmd === "save_project") { saveCalls.push((payload as Record<string, unknown>)?.config); return "proj-saved"; }
      if (cmd === "set_menu_context") return null;
      if (cmd === "list_projects") return [];
      if (cmd === "list_styles") return [];
      if (cmd === "get_provider_status") return [];
      return null;
    });

    useProjectStore.getState().setProjectId("old-proj");
    useProjectStore.getState().setTitle("Old Title");
    useProjectStore.getState().setVideoFile({
      path: "/tmp/v.mp4", name: "v.mp4", size: 100,
      duration: 10, resolution: { width: 640, height: 480 }, codec: "h264", fps: 30,
    });

    render(<App />);
    // Enter editor
    await emitMenuEvent("new_project");
    // ⌘N again triggers dialog
    await emitMenuEvent("new_project");
    // But wait — after doNewProject the stores were reset, so the second ⌘N
    // dialog's Save & New would save an empty project. Let me re-set state.
    // Actually, the first emitMenuEvent("new_project") goes straight to editor
    // because view was "library". Let me set up properly.
    useProjectStore.getState().setTitle("Work to save");
    useProjectStore.getState().setVideoFile({
      path: "/tmp/v2.mp4", name: "v2.mp4", size: 200,
      duration: 20, resolution: { width: 1920, height: 1080 }, codec: "h264", fps: 30,
    });

    await emitMenuEvent("new_project");
    await waitFor(() => { expect(screen.getByText("Save before closing?")).toBeInTheDocument(); });

    await act(async () => { screen.getByText("Save & New").click(); });

    await waitFor(() => expect(saveCalls.length).toBe(1));
    const saved = saveCalls[0] as Record<string, unknown>;
    expect(saved.title).toBe("Work to save");

    // After save, a new project should be created
    expect(useProjectStore.getState().title).toBe("");
  });

  it("save_project preserves created_at for existing projects", async () => {
    const saveCalls: unknown[] = [];
    clearMocks();
    mockIPC((cmd, payload) => {
      if (cmd === "save_project") { saveCalls.push((payload as Record<string, unknown>)?.config); return "proj-saved"; }
      if (cmd === "set_menu_context") return null;
      if (cmd === "list_projects") return [];
      if (cmd === "list_styles") return [];
      if (cmd === "get_provider_status") return [];
      return null;
    });

    useProjectStore.getState().setProjectId("existing-proj");
    useProjectStore.getState().setCreatedAt("2026-01-15T10:00:00Z");
    useProjectStore.getState().setVideoFile({
      path: "/tmp/v.mp4", name: "v.mp4", size: 100,
      duration: 10, resolution: { width: 640, height: 480 }, codec: "h264", fps: 30,
    });

    render(<App />);
    await emitMenuEvent("save_project");

    await waitFor(() => expect(saveCalls.length).toBe(1));
    const saved = saveCalls[0] as Record<string, unknown>;
    expect(saved.created_at).toBe("2026-01-15T10:00:00Z");
    expect(saved.updated_at).not.toBe("2026-01-15T10:00:00Z");
  });

  it("save_project calls IPC and shows success toast", async () => {
    const saveCalls: unknown[] = [];
    clearMocks();
    mockIPC((cmd, payload) => {
      if (cmd === "save_project") { saveCalls.push((payload as Record<string, unknown>)?.config); return "proj-saved"; }
      if (cmd === "set_menu_context") return null;
      if (cmd === "list_projects") return [];
      if (cmd === "list_styles") return [];
      if (cmd === "get_provider_status") return [];
      return null;
    });

    useProjectStore.getState().setProjectId("test-id-123");
    useProjectStore.getState().setTitle("My Video");
    useProjectStore.getState().setVideoFile({
      path: "/tmp/video.mp4", name: "video.mp4", size: 1000,
      duration: 60, resolution: { width: 1920, height: 1080 }, codec: "h264", fps: 30,
    });
    useConfigStore.getState().setStyle("technical");

    render(<App />);
    await emitMenuEvent("save_project");

    await waitFor(() => expect(saveCalls.length).toBe(1));
    const saved = saveCalls[0] as Record<string, unknown>;
    expect(saved.id).toBe("test-id-123");
    expect(saved.style).toBe("technical");
    await waitFor(() => { expect(document.body.textContent).toContain("Project saved"); });
  });

  it("save_project shows error toast when no video", async () => {
    clearMocks();
    mockIPC((cmd) => {
      if (cmd === "set_menu_context") return null;
      if (cmd === "list_projects") return [];
      if (cmd === "list_styles") return [];
      if (cmd === "get_provider_status") return [];
      return null;
    });
    render(<App />);
    await emitMenuEvent("save_project");
    await waitFor(() => { expect(document.body.textContent).toContain("Add a video before saving"); });
  });

  it("open_settings opens the settings panel", async () => {
    render(<App />);
    await emitMenuEvent("open_settings");
    await waitFor(() => { expect(document.body.textContent).toContain("API Keys"); });
  });

  it("narrator_help opens help panel with tips and troubleshooting", async () => {
    render(<App />);
    await emitMenuEvent("narrator_help");
    await waitFor(() => {
      expect(document.body.textContent).toContain("Narrator Help");
      expect(document.body.textContent).toContain("Getting Started");
      expect(document.body.textContent).toContain("Tips");
      expect(document.body.textContent).toContain("Troubleshooting");
    });
  });

  it("toggle_fullscreen toggles window state (Windows F11)", async () => {
    mockWindow.isFullscreen.mockResolvedValue(false);
    mockWindow.setFullscreen.mockClear();
    render(<App />);
    await emitMenuEvent("toggle_fullscreen");
    await waitFor(() => {
      expect(mockWindow.isFullscreen).toHaveBeenCalled();
      expect(mockWindow.setFullscreen).toHaveBeenCalledWith(true);
    });
  });
});
