import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, act, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { UpdateChecker } from "../components/UpdateChecker";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { message } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";

const mockCheck = vi.mocked(check);
const mockRelaunch = vi.mocked(relaunch);
const mockMessage = vi.mocked(message);
const mockListen = vi.mocked(listen);

function createMockUpdate(version = "1.0.0", body = "Bug fixes") {
  return {
    version,
    body,
    date: "2026-04-05",
    currentVersion: "0.1.0",
    downloadAndInstall: vi.fn(async (onProgress?: (event: any) => void) => {
      onProgress?.({ event: "Started", data: { contentLength: 1000 } });
      onProgress?.({ event: "Progress", data: { chunkLength: 500 } });
      onProgress?.({ event: "Progress", data: { chunkLength: 500 } });
      onProgress?.({ event: "Finished", data: {} });
    }),
  };
}

describe("UpdateChecker", () => {
  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.clearAllMocks();
    mockCheck.mockResolvedValue(null);
    mockRelaunch.mockResolvedValue(undefined);
    mockMessage.mockResolvedValue(undefined as any);
    // Default: listen captures the callback but does nothing
    mockListen.mockImplementation(() => Promise.resolve(() => {}));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  // ── Auto-check on startup ──

  it("renders nothing when no update is available (auto-check)", async () => {
    mockCheck.mockResolvedValue(null);
    const { container } = render(<UpdateChecker />);

    // Advance past the 3s startup delay
    await act(async () => { vi.advanceTimersByTime(3500); });

    expect(mockCheck).toHaveBeenCalledTimes(1);
    expect(container.innerHTML).toBe("");
  });

  it("renders nothing initially before auto-check fires", () => {
    const { container } = render(<UpdateChecker />);
    expect(container.innerHTML).toBe("");
  });

  it("does not show dialog on auto-check when no update", async () => {
    mockCheck.mockResolvedValue(null);
    render(<UpdateChecker />);

    await act(async () => { vi.advanceTimersByTime(3500); });

    expect(mockMessage).not.toHaveBeenCalled();
  });

  it("does not show dialog on auto-check when check throws", async () => {
    mockCheck.mockRejectedValue(new Error("404 Not Found"));
    render(<UpdateChecker />);

    await act(async () => { vi.advanceTimersByTime(3500); });

    expect(mockMessage).not.toHaveBeenCalled();
  });

  // ── Update available ──

  it("shows bottom bar when update is available", async () => {
    const mockUpdate = createMockUpdate("2.0.0");
    mockCheck.mockResolvedValue(mockUpdate as any);

    render(<UpdateChecker />);
    await act(async () => { vi.advanceTimersByTime(3500); });

    expect(screen.getByText(/New update available/)).toBeInTheDocument();
    expect(screen.getByText(/v2\.0\.0/)).toBeInTheDocument();
    expect(screen.getByText("Later")).toBeInTheDocument();
    expect(screen.getByText("Install Now")).toBeInTheDocument();
  });

  it("dismisses bar when Later is clicked", async () => {
    const mockUpdate = createMockUpdate();
    mockCheck.mockResolvedValue(mockUpdate as any);
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    render(<UpdateChecker />);
    await act(async () => { vi.advanceTimersByTime(3500); });

    expect(screen.getByText(/New update available/)).toBeInTheDocument();

    await user.click(screen.getByText("Later"));

    expect(screen.queryByText(/New update available/)).not.toBeInTheDocument();
  });

  // ── Download and install ──

  it("shows downloading state when Install Now is clicked", async () => {
    const mockUpdate = createMockUpdate();
    // Make downloadAndInstall hang so we can observe the downloading state
    mockUpdate.downloadAndInstall = vi.fn(() => new Promise(() => {}));
    mockCheck.mockResolvedValue(mockUpdate as any);
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    render(<UpdateChecker />);
    await act(async () => { vi.advanceTimersByTime(3500); });

    await user.click(screen.getByText("Install Now"));

    expect(screen.getByText(/Downloading update/)).toBeInTheDocument();
  });

  it("shows restart prompt after successful download", async () => {
    const mockUpdate = createMockUpdate();
    mockCheck.mockResolvedValue(mockUpdate as any);
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    render(<UpdateChecker />);
    await act(async () => { vi.advanceTimersByTime(3500); });

    await user.click(screen.getByText("Install Now"));

    await waitFor(() => {
      expect(screen.getByText(/Update installed/)).toBeInTheDocument();
    });
    expect(screen.getByText("Restart Now")).toBeInTheDocument();
    expect(screen.getByText("Later")).toBeInTheDocument();
  });

  it("calls relaunch when Restart Now is clicked", async () => {
    const mockUpdate = createMockUpdate();
    mockCheck.mockResolvedValue(mockUpdate as any);
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    render(<UpdateChecker />);
    await act(async () => { vi.advanceTimersByTime(3500); });

    await user.click(screen.getByText("Install Now"));

    await waitFor(() => {
      expect(screen.getByText("Restart Now")).toBeInTheDocument();
    });

    await user.click(screen.getByText("Restart Now"));

    expect(mockRelaunch).toHaveBeenCalledTimes(1);
  });

  it("shows error dialog when download fails", async () => {
    const mockUpdate = createMockUpdate();
    mockUpdate.downloadAndInstall = vi.fn().mockRejectedValue(new Error("Download failed"));
    mockCheck.mockResolvedValue(mockUpdate as any);
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

    render(<UpdateChecker />);
    await act(async () => { vi.advanceTimersByTime(3500); });

    await user.click(screen.getByText("Install Now"));

    await waitFor(() => {
      expect(mockMessage).toHaveBeenCalledWith(
        "Update failed. Please try again later.",
        expect.objectContaining({ kind: "error" })
      );
    });
  });

  // ── Manual check via menu ──

  it("shows native dialog when manual check finds no update", async () => {
    let menuCallback: ((event: any) => void) | null = null;
    mockListen.mockImplementation(async (_event: string, handler: any) => {
      menuCallback = handler;
      return () => {};
    });
    mockCheck.mockResolvedValue(null);

    render(<UpdateChecker />);

    // Simulate menu "Check for Updates..." click
    await act(async () => {
      menuCallback?.({ payload: "check_for_updates" });
    });

    await waitFor(() => {
      expect(mockMessage).toHaveBeenCalledWith(
        "There are currently no updates available.",
        expect.objectContaining({ title: "Narrator", kind: "info" })
      );
    });
  });

  it("shows native dialog when manual check throws error", async () => {
    let menuCallback: ((event: any) => void) | null = null;
    mockListen.mockImplementation(async (_event: string, handler: any) => {
      menuCallback = handler;
      return () => {};
    });
    mockCheck.mockRejectedValue(new Error("network error"));

    render(<UpdateChecker />);

    await act(async () => {
      menuCallback?.({ payload: "check_for_updates" });
    });

    await waitFor(() => {
      expect(mockMessage).toHaveBeenCalledWith(
        "There are currently no updates available.",
        expect.objectContaining({ title: "Narrator", kind: "info" })
      );
    });
  });

  it("shows update bar when manual check finds an update", async () => {
    let menuCallback: ((event: any) => void) | null = null;
    mockListen.mockImplementation(async (_event: string, handler: any) => {
      menuCallback = handler;
      return () => {};
    });
    const mockUpdate = createMockUpdate("3.0.0");
    mockCheck.mockResolvedValue(mockUpdate as any);

    render(<UpdateChecker />);

    await act(async () => {
      menuCallback?.({ payload: "check_for_updates" });
    });

    await waitFor(() => {
      expect(screen.getByText(/New update available/)).toBeInTheDocument();
      expect(screen.getByText(/v3\.0\.0/)).toBeInTheDocument();
    });
    // Should NOT show native dialog when update is found
    expect(mockMessage).not.toHaveBeenCalled();
  });

  // ── Edge cases ──

  it("ignores non-update menu events", async () => {
    let menuCallback: ((event: any) => void) | null = null;
    mockListen.mockImplementation(async (_event: string, handler: any) => {
      menuCallback = handler;
      return () => {};
    });

    render(<UpdateChecker />);

    await act(async () => {
      menuCallback?.({ payload: "open_settings" });
    });

    // check should only have been called by the auto-check timer, not by the menu event
    expect(mockCheck).not.toHaveBeenCalled();
  });

  it("does not double-check if already checking", async () => {
    let menuCallback: ((event: any) => void) | null = null;
    mockListen.mockImplementation(async (_event: string, handler: any) => {
      menuCallback = handler;
      return () => {};
    });
    // Make check hang
    mockCheck.mockImplementation(() => new Promise(() => {}));

    render(<UpdateChecker />);

    // Fire two manual checks rapidly
    await act(async () => {
      menuCallback?.({ payload: "check_for_updates" });
      menuCallback?.({ payload: "check_for_updates" });
    });

    expect(mockCheck).toHaveBeenCalledTimes(1);
  });
});

