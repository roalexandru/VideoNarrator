import { invoke } from "@tauri-apps/api/core";

let telemetryEnabled = true;

// Track consecutive errors per context for retry counting
const errorCounts = new Map<string, number>();

/** Get and increment the consecutive error count for a context. Resets on success via `resetErrorCount`. */
export function getErrorCount(context: string): number {
  const count = (errorCounts.get(context) || 0) + 1;
  errorCounts.set(context, count);
  return count;
}

export function resetErrorCount(context: string): void {
  errorCounts.delete(context);
}

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

/**
 * Track an error event. Strips PII — only sends error type, message, and context.
 * Never sends file paths, API keys, user content, or identifiable information.
 */
export function trackError(
  context: string,
  error: unknown,
  extra?: Record<string, string | number | boolean>,
): void {
  if (!telemetryEnabled) return;

  let errorType = "unknown";
  let errorMessage = "unknown";

  if (error instanceof Error) {
    errorType = error.constructor.name;
    errorMessage = error.message;
  } else if (typeof error === "string") {
    errorType = "string";
    errorMessage = error;
  }

  // Strip potential PII from error messages: file paths, emails, URLs with keys
  errorMessage = errorMessage
    .replace(/[A-Z]:\\[^\s"']+/gi, "[path]")       // Windows paths
    .replace(/\/[^\s"']*\/[^\s"']*/g, "[path]")     // Unix paths
    .replace(/[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+/g, "[email]")
    .replace(/key[=:]\s*\S+/gi, "key=[redacted]")
    .replace(/sk-[a-zA-Z0-9]+/g, "[api_key]")       // OpenAI keys
    .slice(0, 500); // Cap length — 300 was truncating structured API errors

  const retryCount = getErrorCount(context);

  trackEvent("error", {
    context,
    error_type: errorType,
    error_message: errorMessage,
    retry_count: retryCount,
    ...extra,
  });
}
