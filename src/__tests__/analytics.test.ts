import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";
import {
  initTelemetry,
  setTelemetryEnabled,
  trackEvent,
} from "../features/telemetry/analytics";

describe("analytics module", () => {
  let trackedEvents: Array<{ name: string; props: unknown }> = [];

  beforeEach(() => {
    trackedEvents = [];
    mockIPC((cmd, payload) => {
      const p = payload as Record<string, unknown> | undefined;
      switch (cmd) {
        case "get_telemetry_enabled":
          return true;
        case "track_event":
          trackedEvents.push({
            name: p?.name as string,
            props: p?.props,
          });
          return null;
        default:
          return null;
      }
    });
  });

  afterEach(() => {
    clearMocks();
    // Reset to default enabled state
    setTelemetryEnabled(true);
  });

  it("initTelemetry loads the enabled state from backend", async () => {
    await initTelemetry();
    // Should be enabled (mock returns true)
    trackEvent("test_event");
    // Give the fire-and-forget invoke a tick to execute
    await new Promise((r) => setTimeout(r, 10));
    expect(trackedEvents.length).toBe(1);
    expect(trackedEvents[0].name).toBe("test_event");
  });

  it("trackEvent sends name and props to backend", async () => {
    await initTelemetry();
    trackEvent("feature_used", { feature: "export", format: "srt" });
    await new Promise((r) => setTimeout(r, 10));
    expect(trackedEvents.length).toBe(1);
    expect(trackedEvents[0].name).toBe("feature_used");
    expect(trackedEvents[0].props).toEqual({
      feature: "export",
      format: "srt",
    });
  });

  it("trackEvent sends null props when none provided", async () => {
    await initTelemetry();
    trackEvent("app_launched");
    await new Promise((r) => setTimeout(r, 10));
    expect(trackedEvents[0].props).toBeNull();
  });

  it("does not send events when telemetry is disabled", async () => {
    await initTelemetry();
    setTelemetryEnabled(false);

    trackEvent("should_not_send");
    await new Promise((r) => setTimeout(r, 10));
    expect(trackedEvents.length).toBe(0);
  });

  it("resumes sending events when re-enabled", async () => {
    await initTelemetry();
    setTelemetryEnabled(false);
    trackEvent("blocked");
    await new Promise((r) => setTimeout(r, 10));
    expect(trackedEvents.length).toBe(0);

    setTelemetryEnabled(true);
    trackEvent("resumed");
    await new Promise((r) => setTimeout(r, 10));
    expect(trackedEvents.length).toBe(1);
    expect(trackedEvents[0].name).toBe("resumed");
  });

  it("initTelemetry defaults to enabled when backend call fails", async () => {
    clearMocks();
    mockIPC((cmd, payload) => {
      const p = payload as Record<string, unknown> | undefined;
      if (cmd === "get_telemetry_enabled") throw new Error("config missing");
      if (cmd === "track_event") {
        trackedEvents.push({ name: p?.name as string, props: p?.props });
      }
      return null;
    });

    await initTelemetry();
    trackEvent("still_works");
    await new Promise((r) => setTimeout(r, 10));
    expect(trackedEvents.length).toBe(1);
  });
});
