import { invoke } from "@tauri-apps/api/core";

let telemetryEnabled = true;

export async function initTelemetry(): Promise<void> {
  try {
    telemetryEnabled = await invoke<boolean>("get_telemetry_enabled");
  } catch {
    telemetryEnabled = true;
  }
}

export function setTelemetryEnabled(enabled: boolean): void {
  telemetryEnabled = enabled;
}

export function trackEvent(name: string, props?: Record<string, string | number | boolean>): void {
  if (!telemetryEnabled) return;
  invoke("track_event", {
    name,
    props: props ?? null,
  }).catch(() => {});
}
