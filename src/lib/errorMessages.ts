/**
 * Maps raw backend error strings to user-friendly, actionable messages.
 * Each message tells the user WHAT happened and WHAT TO DO about it.
 */
export function toUserMessage(raw: unknown): string {
  const msg = typeof raw === "string" ? raw : raw instanceof Error ? raw.message : String(raw);
  const lower = msg.toLowerCase();

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
    return "Video processing failed. Make sure FFmpeg is up to date (v5+ recommended) and try again.";
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
