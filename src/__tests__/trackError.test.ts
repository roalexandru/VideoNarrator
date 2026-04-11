import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";
import { trackError, trackEvent, setTelemetryEnabled } from "../features/telemetry/analytics";

describe("trackError", () => {
  let ipcCalls: Array<{ cmd: string; payload: Record<string, unknown> }>;

  beforeEach(() => {
    ipcCalls = [];
    setTelemetryEnabled(true);

    mockIPC((cmd, payload) => {
      ipcCalls.push({ cmd, payload: payload as Record<string, unknown> });
      return null;
    });
  });

  afterEach(() => {
    clearMocks();
  });

  // ── Error type detection ──

  it("sends error event with type and message for Error object", () => {
    trackError("test_ctx", new TypeError("something broke"));

    const call = ipcCalls.find((c) => c.cmd === "track_event");
    expect(call).toBeDefined();
    expect(call!.payload.name).toBe("error");
    const props = call!.payload.props as Record<string, unknown>;
    expect(props.error_type).toBe("TypeError");
    expect(props.error_message).toBe("something broke");
    expect(props.context).toBe("test_ctx");
  });

  it("sends type 'string' for string errors", () => {
    trackError("test_ctx", "plain string error");

    const call = ipcCalls.find((c) => c.cmd === "track_event");
    const props = call!.payload.props as Record<string, unknown>;
    expect(props.error_type).toBe("string");
    expect(props.error_message).toBe("plain string error");
  });

  it("sends type 'unknown' for non-Error, non-string errors", () => {
    trackError("test_ctx", 42);

    const call = ipcCalls.find((c) => c.cmd === "track_event");
    const props = call!.payload.props as Record<string, unknown>;
    expect(props.error_type).toBe("unknown");
    expect(props.error_message).toBe("unknown");
  });

  // ── PII stripping ──

  it("strips Windows paths from error messages", () => {
    trackError("test_ctx", new Error("Failed to read C:\\Users\\John\\Documents\\secret.txt"));

    const call = ipcCalls.find((c) => c.cmd === "track_event");
    const props = call!.payload.props as Record<string, unknown>;
    expect(props.error_message).not.toContain("C:\\Users");
    expect(props.error_message).toContain("[path]");
  });

  it("strips Unix paths from error messages", () => {
    trackError("test_ctx", new Error("Failed to read /home/user/documents/secret.txt"));

    const call = ipcCalls.find((c) => c.cmd === "track_event");
    const props = call!.payload.props as Record<string, unknown>;
    expect(props.error_message).not.toContain("/home/user");
    expect(props.error_message).toContain("[path]");
  });

  it("strips email addresses from error messages", () => {
    trackError("test_ctx", new Error("Error for user john.doe@example.com"));

    const call = ipcCalls.find((c) => c.cmd === "track_event");
    const props = call!.payload.props as Record<string, unknown>;
    expect(props.error_message).not.toContain("john.doe@example.com");
    expect(props.error_message).toContain("[email]");
  });

  it("strips OpenAI API keys (sk-...)", () => {
    trackError("test_ctx", new Error("Auth failed with token sk-abc123XYZ456 on request"));

    const call = ipcCalls.find((c) => c.cmd === "track_event");
    const props = call!.payload.props as Record<string, unknown>;
    expect(props.error_message).not.toContain("sk-abc123XYZ456");
    expect(props.error_message).toContain("[api_key]");
  });

  it("strips key= patterns", () => {
    trackError("test_ctx", new Error("Request with key=super_secret_value failed"));

    const call = ipcCalls.find((c) => c.cmd === "track_event");
    const props = call!.payload.props as Record<string, unknown>;
    expect(props.error_message).not.toContain("super_secret_value");
    expect(props.error_message).toContain("[redacted]");
  });

  // ── Message length cap ──

  it("caps error message at 500 characters", () => {
    const longMessage = "A".repeat(800);
    trackError("test_ctx", new Error(longMessage));

    const call = ipcCalls.find((c) => c.cmd === "track_event");
    const props = call!.payload.props as Record<string, unknown>;
    expect((props.error_message as string).length).toBeLessThanOrEqual(500);
  });

  // ── Extra props ──

  it("passes extra props through", () => {
    trackError("test_ctx", new Error("fail"), { step: "export", attempt: 3 });

    const call = ipcCalls.find((c) => c.cmd === "track_event");
    const props = call!.payload.props as Record<string, unknown>;
    expect(props.step).toBe("export");
    expect(props.attempt).toBe(3);
  });

  // ── Telemetry enabled/disabled ──

  it("does not send events when telemetry is disabled", () => {
    setTelemetryEnabled(false);
    trackError("test_ctx", new Error("should not send"));

    const call = ipcCalls.find((c) => c.cmd === "track_event");
    expect(call).toBeUndefined();
  });

  it("sends events when telemetry is re-enabled", () => {
    setTelemetryEnabled(false);
    trackError("test_ctx", new Error("should not send"));

    setTelemetryEnabled(true);
    trackError("test_ctx", new Error("should send"));

    const calls = ipcCalls.filter((c) => c.cmd === "track_event");
    expect(calls).toHaveLength(1);
  });
});

describe("trackEvent telemetry gating", () => {
  let ipcCalls: Array<{ cmd: string; payload: Record<string, unknown> }>;

  beforeEach(() => {
    ipcCalls = [];
    setTelemetryEnabled(true);

    mockIPC((cmd, payload) => {
      ipcCalls.push({ cmd, payload: payload as Record<string, unknown> });
      return null;
    });
  });

  afterEach(() => {
    clearMocks();
  });

  it("does not send trackEvent when telemetry is disabled", () => {
    setTelemetryEnabled(false);
    trackEvent("some_event", { foo: "bar" });

    const call = ipcCalls.find((c) => c.cmd === "track_event");
    expect(call).toBeUndefined();
  });
});
