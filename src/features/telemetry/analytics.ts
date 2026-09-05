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
  // Aptabase has a first-class `locale` column that was being sent empty,
  // because Rust has no portable way to read the UI language. The webview
  // does, so hand it over once. Fire-and-forget: a failure here costs one
  // analytics dimension, not the session.
  try {
    const locale = navigator.language;
    if (locale) await invoke("set_telemetry_locale", { locale });
  } catch {
    // ignored
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

  // Strip potential PII from error messages: file paths, emails, URLs with keys.
  //
  // `error_message` forwards arbitrary upstream provider text, so the redaction
  // has to cover every vendor's key shape, not just OpenAI's `sk-` prefix.
  // Gemini keys (`AIza…`), and Azure/ElevenLabs keys (bare 32-char hex) carry
  // no prefix at all, and a JSON-quoted field like `"xi-api-key": "abc"` slips
  // past a `key[=:]` pattern because the quote sits between the two.
  errorMessage = errorMessage
    .replace(/[A-Z]:\\[^\s"']+/gi, "[path]")       // Windows paths
    .replace(/\/[^\s"']*\/[^\s"']*/g, "[path]")     // Unix paths
    .replace(/[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+/g, "[email]")
    // Quoted JSON key fields, e.g. "api-key": "value" / "xi-api-key":"value".
    .replace(/"[^"\s]*(?:key|token|secret)"\s*:\s*"[^"]*"/gi, '"[redacted]":"[redacted]"')
    .replace(/(key|token|secret)[=:]\s*\S+/gi, "$1=[redacted]")
    .replace(/\bBearer\s+\S+/gi, "Bearer [redacted]")
    .replace(/sk-[a-zA-Z0-9_-]+/g, "[api_key]")     // OpenAI / Anthropic
    .replace(/AIza[0-9A-Za-z_-]{20,}/g, "[api_key]") // Google / Gemini
    // Azure and ElevenLabs keys are bare hex with no prefix to anchor on.
    .replace(/\b[0-9a-f]{32,}\b/gi, "[api_key]")
    .slice(0, 500); // Cap length — 300 was truncating structured API errors

  // Consecutive failures for this context, reset by `resetErrorCount` on the
  // next success. Deliberately NOT called `retry_count`: nothing here retries,
  // and the old name read as "we retried N times" when it means "this is the
  // Nth failure in a row" — which inverts how you'd judge an error's severity.
  const consecutiveFailures = getErrorCount(context);

  trackEvent("error", {
    context,
    error_type: errorType,
    error_message: errorMessage,
    consecutive_failures: consecutiveFailures,
    ...extra,
  });
}
