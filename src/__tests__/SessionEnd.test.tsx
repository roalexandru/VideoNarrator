/**
 * Guard for the worst of the telemetry-instrumentation defects found by
 * auditing a production Aptabase export.
 *
 * `session_end` used to fire from `visibilitychange -> hidden` behind a
 * one-shot latch. On macOS that fires on minimize / Cmd+H / occlusion, so the
 * latch froze `duration_seconds` at however long the user had been in the app
 * before they first switched away. Observed in the export: a session reported
 * `duration_seconds: 42` and then produced sixty more events over the next
 * five and a half hours, all under the same `session_id`.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, act } from "@testing-library/react";
import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";
import { resetAllStores, setupDefaultMocks } from "./setup";
import App from "../App";

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));

let closeHandler: (() => void) | null = null;

const mockWindow = {
  minimize: vi.fn(),
  close: vi.fn(),
  setFocus: vi.fn(),
  startDragging: vi.fn(),
  unminimize: vi.fn(),
  isFullscreen: vi.fn().mockResolvedValue(false),
  setFullscreen: vi.fn(),
  onCloseRequested: vi.fn((handler: () => void) => {
    closeHandler = handler;
    return Promise.resolve(() => {});
  }),
};

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => mockWindow,
}));

/** Drive `document.visibilityState`, which is otherwise read-only. */
function setVisibility(state: "visible" | "hidden") {
  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    get: () => state,
  });
  document.dispatchEvent(new Event("visibilitychange"));
}

describe("session_end", () => {
  let events: Array<{ name: string; props: Record<string, unknown> }>;

  beforeEach(() => {
    resetAllStores();
    setupDefaultMocks();
    closeHandler = null;
    events = [];
    // Layer over the default mocks so every other command still resolves.
    const base = (cmd: string) => {
      // Must be true, or `trackEvent` short-circuits and every assertion in
      // this file passes for the wrong reason.
      if (cmd === "get_telemetry_enabled") return true;
      if (cmd === "list_projects" || cmd === "list_styles" || cmd === "get_provider_status") return [];
      return null;
    };
    mockIPC((cmd, payload) => {
      if (cmd === "track_event") {
        const p = payload as { name: string; props: Record<string, unknown> };
        events.push({ name: p.name, props: p.props ?? {} });
        return null;
      }
      return base(cmd);
    });
  });

  afterEach(() => {
    clearMocks();
    setVisibility("visible");
  });

  const sessionEnds = () => events.filter((e) => e.name === "session_end");

  it("does not fire when the window is merely hidden", async () => {
    render(<App />);
    setVisibility("hidden");
    await act(async () => {});
    expect(sessionEnds()).toHaveLength(0);
  });

  it("still does not fire after repeated hide/show cycles", async () => {
    render(<App />);
    for (let i = 0; i < 3; i++) {
      setVisibility("hidden");
      await act(async () => {});
      setVisibility("visible");
      await act(async () => {});
    }
    expect(sessionEnds()).toHaveLength(0);
  });

  it("fires once when the window is actually closing", async () => {
    render(<App />);
    await act(async () => {});
    expect(closeHandler).not.toBeNull();
    await act(async () => {
      closeHandler!();
    });
    expect(sessionEnds()).toHaveLength(1);
    const props = sessionEnds()[0].props;
    expect(typeof props.duration_seconds).toBe("number");
    // Reported separately so a session parked in the background is
    // distinguishable from one actively used for the same wall time.
    expect(typeof props.active_seconds).toBe("number");
  });

  it("does not double-report when close follows a hide", async () => {
    render(<App />);
    await act(async () => {});
    expect(closeHandler).not.toBeNull();
    setVisibility("hidden");
    await act(async () => {});
    await act(async () => {
      closeHandler!();
    });
    // And a second close signal must not add another row.
    await act(async () => {
      closeHandler!();
    });
    expect(sessionEnds()).toHaveLength(1);
  });

  it("counts background time in duration but not in active time", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      render(<App />);
      await act(async () => {});
      expect(closeHandler).not.toBeNull();

      // 10s on screen, then 60s hidden, then close.
      await act(async () => {
        vi.advanceTimersByTime(10_000);
      });
      setVisibility("hidden");
      await act(async () => {
        vi.advanceTimersByTime(60_000);
      });
      await act(async () => {
        closeHandler!();
      });

      const props = sessionEnds()[0].props;
      expect(props.duration_seconds as number).toBeGreaterThanOrEqual(65);
      // The 60s hidden stretch must not count as active — this is the number
      // the old code would have frozen at ~0.
      expect(props.active_seconds as number).toBeLessThan(20);
      expect(props.active_seconds as number).toBeGreaterThanOrEqual(9);
    } finally {
      vi.useRealTimers();
    }
  });
});
