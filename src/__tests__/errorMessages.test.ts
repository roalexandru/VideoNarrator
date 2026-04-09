import { describe, it, expect } from "vitest";
import { toUserMessage } from "../lib/errorMessages";

describe("toUserMessage", () => {
  // ── API key / auth errors ──

  describe("no API key errors", () => {
    it("returns generic settings direction for 'no api key'", () => {
      const result = toUserMessage("No API key configured");
      expect(result).toContain("Settings");
      expect(result).toContain("API key");
    });

    it("returns ElevenLabs direction for elevenlabs no api key", () => {
      const result = toUserMessage("No API key for ElevenLabs");
      expect(result).toContain("ElevenLabs");
      expect(result).toContain("Settings");
    });

    it("returns Azure direction for azure no api key", () => {
      const result = toUserMessage("No API key for Azure TTS");
      expect(result).toContain("Azure");
      expect(result).toContain("Settings");
    });

    it("handles NoApiKey variant", () => {
      const result = toUserMessage("NoApiKey");
      expect(result).toContain("Settings");
      expect(result).toContain("API key");
    });
  });

  describe("401/unauthorized errors", () => {
    it("returns generic auth message for plain 401", () => {
      const result = toUserMessage("HTTP 401 Unauthorized");
      expect(result).toContain("Authentication failed");
      expect(result).toContain("Settings");
    });

    it("returns Claude-specific message for claude 401", () => {
      const result = toUserMessage("401 unauthorized from Claude API");
      expect(result).toContain("Claude");
      expect(result).toContain("Settings");
      expect(result).toContain("Anthropic");
    });

    it("returns OpenAI-specific message for openai 401", () => {
      const result = toUserMessage("401 unauthorized from OpenAI");
      expect(result).toContain("OpenAI");
      expect(result).toContain("Settings");
    });

    it("returns Gemini-specific message for gemini 401", () => {
      const result = toUserMessage("401 unauthorized from Gemini");
      expect(result).toContain("Gemini");
      expect(result).toContain("Settings");
    });

    it("returns ElevenLabs-specific message for elevenlabs 401", () => {
      const result = toUserMessage("401 unauthorized from ElevenLabs");
      expect(result).toContain("ElevenLabs");
      expect(result).toContain("Settings");
    });

    it("returns Azure-specific message for azure 401", () => {
      const result = toUserMessage("401 unauthorized from Azure TTS");
      expect(result).toContain("Azure");
      expect(result).toContain("Settings");
    });
  });

  // ── Rate limiting ──

  describe("rate limiting", () => {
    it("returns wait/retry message for 429", () => {
      const result = toUserMessage("HTTP 429");
      expect(result).toContain("rate limiting");
      expect(result).toContain("Wait");
    });

    it("returns wait/retry message for 'rate limit'", () => {
      const result = toUserMessage("rate limit exceeded");
      expect(result).toContain("rate limiting");
    });

    it("returns wait/retry message for 'too many requests'", () => {
      const result = toUserMessage("Too many requests");
      expect(result).toContain("rate limiting");
    });
  });

  // ── Network errors ──

  describe("network errors", () => {
    it("returns check internet message for network error", () => {
      const result = toUserMessage("network error");
      expect(result).toContain("internet");
    });

    it("returns check internet message for connection error", () => {
      const result = toUserMessage("connection refused");
      expect(result).toContain("internet");
    });

    it("returns check internet message for DNS error", () => {
      const result = toUserMessage("DNS resolution failed");
      expect(result).toContain("internet");
    });

    it("returns check internet message for timeout", () => {
      const result = toUserMessage("request timed out");
      expect(result).toContain("internet");
    });
  });

  // ── Server errors ──

  describe("server errors (500/503)", () => {
    it("returns try again message for 500", () => {
      const result = toUserMessage("HTTP 500 internal server error");
      expect(result).toContain("temporarily unavailable");
      expect(result).toContain("switch");
    });

    it("returns try again message for 503", () => {
      const result = toUserMessage("503 service unavailable");
      expect(result).toContain("temporarily unavailable");
    });

    it("returns try again message for generic server error", () => {
      const result = toUserMessage("server error occurred");
      expect(result).toContain("temporarily unavailable");
    });
  });

  // ── ffmpeg ──

  describe("ffmpeg errors", () => {
    it("returns install instructions for ffmpeg not found", () => {
      const result = toUserMessage("ffmpeg not found");
      expect(result).toContain("brew install ffmpeg");
      expect(result).toContain("choco install ffmpeg");
      expect(result).toContain("restart");
    });

    it("handles FfmpegNotFound variant", () => {
      const result = toUserMessage("FfmpegNotFound");
      expect(result).toContain("brew install ffmpeg");
    });

    it("returns update message for ffmpeg failed", () => {
      const result = toUserMessage("ffmpeg failed to process");
      expect(result).toContain("FFmpeg");
      expect(result).toContain("up to date");
    });
  });

  // ── Video probe errors ──

  describe("video probe errors", () => {
    it("returns validity message for video probe error", () => {
      const result = toUserMessage("video probe failed");
      expect(result).toContain("valid video");
      expect(result).toContain("MP4");
    });

    it("returns validity message for no video stream", () => {
      const result = toUserMessage("no video stream found");
      expect(result).toContain("valid video");
    });
  });

  // ── Document processing ──

  describe("document processing errors", () => {
    it("returns supported formats for unsupported document", () => {
      const result = toUserMessage("unsupported document type");
      expect(result).toContain(".md");
      expect(result).toContain(".txt");
      expect(result).toContain(".pdf");
    });

    it("returns convert suggestion for PDF extraction failure", () => {
      const result = toUserMessage("Could not extract text from PDF");
      expect(result).toContain("converting");
    });
  });

  // ── TTS specific ──

  describe("audio generation failed", () => {
    it("passes through enriched audio generation message", () => {
      const msg = "Audio generation failed: voice not found";
      const result = toUserMessage(msg);
      expect(result).toBe(msg);
    });
  });

  // ── Parse errors ──

  describe("parse errors", () => {
    it("returns try different model for parse error", () => {
      const result = toUserMessage("Failed to parse AI response");
      expect(result).toContain("invalid response");
      expect(result).toContain("different AI model");
    });

    it("returns try different model for translation parse error", () => {
      const result = toUserMessage("Failed to parse translation response");
      expect(result).toContain("invalid response");
    });
  });

  // ── Cancelled ──

  describe("cancelled", () => {
    it("returns simple cancelled message", () => {
      const result = toUserMessage("Operation cancelled by user");
      expect(result).toBe("Operation was cancelled.");
    });

    it("handles 'canceled' (American spelling)", () => {
      const result = toUserMessage("request was canceled");
      expect(result).toBe("Operation was cancelled.");
    });
  });

  // ── Rate limiter wait ──

  describe("rate limiter wait message", () => {
    it("returns wait message", () => {
      const result = toUserMessage("Please wait before validating another key");
      expect(result).toContain("wait");
    });
  });

  // ── Quota / billing ──

  describe("quota/billing errors", () => {
    it("returns quota message for 402", () => {
      const result = toUserMessage("HTTP 402 payment required");
      expect(result).toContain("quota");
    });

    it("returns billing message for quota exceeded", () => {
      const result = toUserMessage("quota exceeded for this account");
      expect(result).toContain("billing");
    });
  });

  // ── Unknown / fallback ──

  describe("unknown errors", () => {
    it("returns original message when short", () => {
      const msg = "Something weird happened";
      expect(toUserMessage(msg)).toBe(msg);
    });

    it("caps original message at 200 chars", () => {
      const longMsg = "A".repeat(300);
      const result = toUserMessage(longMsg);
      expect(result.length).toBe(201); // 200 + "…"
      expect(result.endsWith("\u2026")).toBe(true);
    });

    it("returns original when exactly 200 chars", () => {
      const msg = "A".repeat(200);
      expect(toUserMessage(msg)).toBe(msg);
    });
  });

  // ── Input handling ──

  describe("input types", () => {
    it("handles Error objects", () => {
      const error = new Error("401 unauthorized from OpenAI");
      const result = toUserMessage(error);
      expect(result).toContain("OpenAI");
    });

    it("handles non-string non-Error input", () => {
      const result = toUserMessage(42);
      expect(result).toBe("42");
    });

    it("handles null-ish input via String()", () => {
      const result = toUserMessage(undefined);
      expect(result).toBe("undefined");
    });
  });
});
