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
    lower.includes("413 ") ||
    lower.includes(" 413") ||
    lower.includes("request entity too large")
  );
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

  // ── API key / auth errors ──
  if (lower.includes("no api key") || lower.includes("noapikey")) {
    if (lower.includes("elevenlabs"))
      return "No ElevenLabs API key configured. Go to Settings → Voice → ElevenLabs and add your key.";
    if (lower.includes("azure"))
      return "No Azure TTS API key configured. Go to Settings → Voice → Azure TTS and add your key and region.";
    return "No API key configured. Go to Settings and add your API key for the selected provider.";
  }

  if (lower.includes("401") || lower.includes("unauthorized") || lower.includes("invalid.*key")) {
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

  // ── Rate limiting ──
  if (lower.includes("429") || lower.includes("rate limit") || lower.includes("too many requests")) {
    return "Too many requests — the API provider is rate limiting you. Wait a moment and try again.";
  }

  // ── Quota / billing ──
  if (lower.includes("402") || lower.includes("quota") || lower.includes("insufficient") || lower.includes("billing")) {
    return "API quota exceeded or billing issue. Check your account balance with the provider.";
  }

  // ── Network errors ──
  if (lower.includes("network") || lower.includes("connect") || lower.includes("dns") || lower.includes("timed out") || lower.includes("timeout")) {
    return "Network connection failed. Check your internet connection and try again.";
  }

  // ── Server errors ──
  if (lower.includes("500") || lower.includes("502") || lower.includes("503") || lower.includes("server error")) {
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
