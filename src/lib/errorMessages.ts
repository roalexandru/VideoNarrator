/**
 * True when the error looks like the request payload exceeded the model's
 * context / token limit. Different providers word this very differently —
 * Claude uses 400 with "prompt is too long", OpenAI uses "context_length_exceeded"
 * or "maximum context length", Gemini uses "overloaded_token_input" or a
 * 400 with "exceeds the maximum number of tokens". A plain 413 also qualifies.
 *
 * Retrying the same request will always fail, so callers use this to skip
 * the rate-limit cooldown (retry after 30s is pointless here) and to show a
 * different, actionable error message.
 */
export function isContextOverflowError(raw: unknown): boolean {
  const msg = typeof raw === "string" ? raw : raw instanceof Error ? raw.message : String(raw);
  const lower = msg.toLowerCase();
  return (
    lower.includes("context length") ||
    lower.includes("context window") ||
    lower.includes("context_length_exceeded") ||
    lower.includes("maximum context") ||
    lower.includes("prompt is too long") ||
    lower.includes("too many tokens") ||
    lower.includes("token limit") ||
    lower.includes("tokens_exceed") ||
    lower.includes("overloaded_token_input") ||
    lower.includes("payload too large") ||
    /(?<!\d)413(?!\d)/.test(lower) ||
    lower.includes("request entity too large")
  );
}

/**
 * True when the error is a billing / credit / quota problem rather than a
 * transient rate limit. This is PERMANENT until the user adds credit or fixes
 * billing, so — like context overflow — callers must skip the rate-limit
 * cooldown (a "wait 30s and retry" loop can never succeed) and show a
 * different, actionable message. Providers are inconsistent: Anthropic says
 * "credit balance too low", OpenAI returns a 429 with "insufficient_quota",
 * and our backend wraps both as "API credit or billing problem: …".
 */
export function isBillingError(raw: unknown): boolean {
  const msg = typeof raw === "string" ? raw : raw instanceof Error ? raw.message : String(raw);
  const lower = msg.toLowerCase();
  return (
    lower.includes("credit or billing problem") ||
    lower.includes("credit balance") ||
    lower.includes("insufficient_quota") ||
    lower.includes("insufficient quota") ||
    lower.includes("insufficient credit") ||
    lower.includes("out of credits") ||
    lower.includes("exceeded your current quota") ||
    lower.includes("purchase credits") ||
    lower.includes("quota") ||
    lower.includes("billing") ||
    hasHttpStatus(lower, [402])
  );
}

/**
 * True when `lower` contains any of the given HTTP status codes as an
 * isolated number — i.e. with non-digit boundaries on either side. Without
 * this, a backend message like "Description must be 5000 characters or fewer"
 * would match `"500"` and get misclassified as a 5xx server error, and "1500"
 * would match too.
 */
function hasHttpStatus(lower: string, codes: number[]): boolean {
  return codes.some((code) => new RegExp(`(?<!\\d)${code}(?!\\d)`).test(lower));
}

/**
 * Maps raw backend error strings to user-friendly, actionable messages.
 * Each message tells the user WHAT happened and WHAT TO DO about it.
 */
export function toUserMessage(raw: unknown): string {
  const msg = typeof raw === "string" ? raw : raw instanceof Error ? raw.message : String(raw);
  const lower = msg.toLowerCase();

  // ── Context / token overflow ── (check before generic 4xx so "413" routes here)
  if (isContextOverflowError(raw)) {
    return "Request exceeds the model's context window. Try lowering Frame Extraction density, removing context documents, or switching to a larger-context model in Settings.";
  }

  // ── Backend input-validation messages ──
  // These contain digits (e.g. "5000 characters") that would otherwise be
  // misclassified by the status-code branches below as 5xx server errors.
  // The backend message is already specific and actionable, so pass through.
  if (lower.includes("characters or fewer") || lower.includes("size exceeds")) {
    return msg;
  }

  // ── API key / auth errors ──
  if (lower.includes("no api key") || lower.includes("noapikey")) {
    if (lower.includes("elevenlabs"))
      return "No ElevenLabs API key configured. Go to Settings → Voice → ElevenLabs and add your key.";
    if (lower.includes("azure"))
      return "No Azure TTS API key configured. Go to Settings → Voice → Azure TTS and add your key and region.";
    return "No API key configured. Go to Settings and add your API key for the selected provider.";
  }

  if (hasHttpStatus(lower, [401]) || lower.includes("unauthorized")) {
    if (lower.includes("azure"))
      return "Azure TTS authentication failed. Check your API key and region in Settings → Voice → Azure TTS.";
    if (lower.includes("elevenlabs"))
      return "ElevenLabs authentication failed. Check your API key in Settings → Voice → ElevenLabs.";
    if (lower.includes("claude") || lower.includes("anthropic"))
      return "Claude API key is invalid or expired. Update it in Settings → AI Providers → Anthropic Claude.";
    if (lower.includes("openai"))
      return "OpenAI API key is invalid or expired. Update it in Settings → AI Providers → OpenAI.";
    if (lower.includes("gemini"))
      return "Gemini API key is invalid or expired. Update it in Settings → AI Providers → Google Gemini.";
    return "Authentication failed. Check your API key in Settings.";
  }

  // ── Quota / billing ── (checked BEFORE rate limiting: providers often
  // surface a no-credit account as a 429 with "insufficient_quota", and this
  // is permanent — routing it to the rate-limit "wait and retry" message sends
  // users in circles.)
  if (isBillingError(raw)) {
    // The backend builds an actionable, provider-labelled message for these
    // ("API credit or billing problem: …") — surface it directly rather than
    // flattening it to something vaguer.
    if (lower.includes("credit or billing problem")) {
      return msg.replace(/^api credit or billing problem:\s*/i, "").trim();
    }
    return "Your API account is out of credit or has a billing problem. Add credit or update billing in your provider's console, then try again — waiting won't help.";
  }

  // ── Rate limiting ──
  if (hasHttpStatus(lower, [429]) || lower.includes("rate limit") || lower.includes("too many requests")) {
    return "Too many requests — the API provider is rate limiting you. Wait a moment and try again.";
  }

  // ── Network errors ──
  if (lower.includes("network") || lower.includes("connect") || lower.includes("dns") || lower.includes("timed out") || lower.includes("timeout")) {
    return "Network connection failed. Check your internet connection and try again.";
  }

  // ── Server errors ──
  if (hasHttpStatus(lower, [500, 502, 503, 504]) || lower.includes("server error") || lower.includes("bad gateway") || lower.includes("service unavailable") || lower.includes("gateway timeout")) {
    return "The API server is temporarily unavailable. Try again in a few seconds, or switch to a different provider in Settings.";
  }

  // ── ffmpeg ──
  if (lower.includes("ffmpeg not found") || lower.includes("ffmpegnotfound")) {
    return "FFmpeg is required but not installed. Install it:\n• macOS: brew install ffmpeg\n• Windows: choco install ffmpeg\n• Linux: sudo apt install ffmpeg\nThen restart Narrator.";
  }
  if (lower.includes("ffmpeg failed") || lower.includes("ffmpegfailed")) {
    // Show the actual ffmpeg error detail so users (and developers) can diagnose
    const detail = msg.replace(/^.*?ffmpeg\s*(failed)?:?\s*/i, "").trim();
    return `Video processing failed${detail ? `: ${detail}` : ""}. Make sure FFmpeg is up to date (v5+ recommended).`;
  }

  // ── Video probe ──
  if (lower.includes("video probe") || lower.includes("no video stream")) {
    return "Could not read this video file. Make sure it's a valid video (MP4, MOV, MKV, AVI, WebM) and not corrupted.";
  }

  // ── Document processing ──
  if (lower.includes("unsupported document")) {
    return "Unsupported file type. Only .md, .txt, and .pdf documents are supported.";
  }
  if (lower.includes("could not extract text from pdf")) {
    return "Could not extract text from this PDF. Try converting it to .txt or .md first.";
  }

  // ── TTS specific ──
  if (lower.includes("audio generation failed")) {
    return msg; // Already enriched with the specific TTS error
  }

  // ── Parse errors ──
  if (lower.includes("failed to parse ai response") || lower.includes("failed to parse translation")) {
    return "The AI returned an invalid response. Try again — if this persists, try a different AI model in Settings.";
  }

  // ── Cancelled ──
  if (lower.includes("cancelled") || lower.includes("canceled")) {
    return "Operation was cancelled.";
  }

  // ── Wait message from rate limiter ──
  if (lower.includes("please wait before validating")) {
    return "Please wait a moment before validating another key.";
  }

  // ── Fallback: return original but cap length ──
  return msg.length > 200 ? msg.slice(0, 200) + "…" : msg;
}
