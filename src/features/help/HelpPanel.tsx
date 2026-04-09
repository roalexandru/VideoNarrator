import { useState, useEffect, useCallback } from "react";
import { colors, typography } from "../../lib/theme";
import { Button } from "../../components/ui/Button";
import { checkFfmpeg } from "../../lib/tauri/commands";
import { trackEvent } from "../telemetry/analytics";

const isMac = navigator.platform.toUpperCase().includes("MAC");

const sections = [
  {
    title: "Getting Started",
    items: [
      {
        q: "What is Narrator?",
        a: "Narrator is an AI-powered video narration generator. Import a video or record your screen, and Narrator will analyze the visual content frame-by-frame to produce a professional narration script. You can then export the script in multiple formats or generate speech audio with ElevenLabs or Azure TTS.",
      },
      {
        q: "Prerequisites",
        a: "FFmpeg must be installed on your system for video processing. You will also need at least one AI provider API key (Anthropic Claude, OpenAI, or Google Gemini) configured in Settings. For text-to-speech, an ElevenLabs or Azure TTS API key is required.",
      },
      {
        q: "Quick start",
        a: `1. Open Settings (${isMac ? "⌘," : "Ctrl+,"}) and add your API key.\n2. Click New Project and import a video file.\n3. Optionally trim or split the video in the Edit step.\n4. Choose a narration style, language, and AI provider.\n5. Click Generate to create the narration.\n6. Review and edit the script, then export.`,
      },
    ],
  },
  {
    title: "Tips",
    items: [
      {
        q: "Better narration results",
        a: "Attach context documents (Markdown, TXT, PDF) in the Setup step — they give the AI background about your content. For technical demos, the \"Technical Deep-Dive\" style produces more precise descriptions. Lower the temperature (0.3–0.5) for consistent output, raise it (0.8+) for creative variety.",
      },
      {
        q: "Working with long videos",
        a: "Use the Edit step to split long videos into focused clips. Increase frame density to \"Heavy\" in Configuration so the AI captures more visual detail. You can always cancel generation mid-way if you want to adjust settings.",
      },
      {
        q: "Multi-language narration",
        a: "Select multiple languages in Configuration before generating. The primary language is generated first from video frames; additional languages are translated from it. You can switch between languages in the Review step to edit each independently.",
      },
      {
        q: "Auto-updates",
        a: "Narrator checks for updates automatically on launch. You can also check manually via the app menu (Help > Check for Updates). When an update is available, a bar appears at the bottom of the window with an option to install.",
      },
      {
        q: "Export options",
        a: "The Export step offers three outputs:\n• Video — burn subtitles directly onto the video, or merge with TTS audio\n• Audio Only — generate narration audio via ElevenLabs or Azure TTS\n• Scripts — export as SRT (subtitles), VTT (web video), JSON (programmatic), Markdown (readable), or SSML (speech synthesis)",
      },
    ],
  },
  {
    title: "Keyboard Shortcuts",
    items: [
      {
        q: "General",
        a: isMac
          ? "New Project — ⌘N\nSave Project — ⌘S\nSettings — ⌘,"
          : "New Project — Ctrl+N\nSave Project — Ctrl+S\nSettings — Ctrl+,\nFull Screen — F11",
      },
      {
        q: "Text editing",
        a: isMac
          ? "Undo — ⌘Z\nRedo — ⌘⇧Z\nCut — ⌘X\nCopy — ⌘C\nPaste — ⌘V\nSelect All — ⌘A"
          : "Undo — Ctrl+Z\nRedo — Ctrl+Shift+Z\nCut — Ctrl+X\nCopy — Ctrl+C\nPaste — Ctrl+V\nSelect All — Ctrl+A",
      },
    ],
  },
  {
    title: "Troubleshooting",
    items: [
      {
        q: "FFmpeg not found",
        a: "Narrator requires FFmpeg for video processing. Install it via your package manager:\n• macOS: brew install ffmpeg\n• Windows: choco install ffmpeg or download from ffmpeg.org\n• Linux: sudo apt install ffmpeg",
        action: "check_ffmpeg",
      },
      {
        q: "API key errors",
        a: `Open Settings (${isMac ? "⌘," : "Ctrl+,"}) and verify your API key is correct. Keys are validated when saved. Make sure you have billing enabled on your provider account and sufficient quota.`,
      },
      {
        q: "Generation fails or produces poor results",
        a: "Try a different narration style or lower the temperature for more consistent output. Adding context documents helps the AI understand domain-specific content. For long videos, a higher frame density captures more visual detail.",
      },
      {
        q: "Audio/video merge issues",
        a: "Ensure FFmpeg is up to date (version 5+ recommended). The merge operation requires the generated audio segments to match the video timeline. If segments are missing, regenerate TTS for the affected segments.",
      },
    ],
  },
];

export function HelpPanel({ onClose, onShowPrivacyPolicy, onShowTerms }: { onClose: () => void; onShowPrivacyPolicy?: () => void; onShowTerms?: () => void }) {
  const [expanded, setExpanded] = useState<string | null>(sections[0].items[0].q);
  const [ffmpegStatus, setFfmpegStatus] = useState<string | null>(null);
  const [feedbackText, setFeedbackText] = useState("");
  const [feedbackType, setFeedbackType] = useState<"bug" | "feature" | "general">("general");
  const [feedbackSent, setFeedbackSent] = useState(false);

  const handleKeyDown = useCallback((e: KeyboardEvent) => {
    if (e.key === "Escape") onClose();
  }, [onClose]);

  useEffect(() => {
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [handleKeyDown]);

  const toggle = (q: string) => setExpanded((prev) => (prev === q ? null : q));

  const handleCheckFfmpeg = async () => {
    try {
      const path = await checkFfmpeg();
      setFfmpegStatus(`Found: ${path}`);
    } catch {
      setFfmpegStatus("Not found — please install FFmpeg");
    }
  };

  return (
    <div
      style={{
        position: "fixed", inset: 0, zIndex: 100,
        background: "rgba(0,0,0,0.6)", backdropFilter: "blur(8px)",
        display: "flex", alignItems: "center", justifyContent: "center",
      }}
      onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}
    >
      <div style={{
        background: colors.bg.card, borderRadius: 14,
        border: `1px solid ${colors.border.default}`,
        boxShadow: "0 32px 64px rgba(0,0,0,0.5)",
        padding: "28px 32px", width: 560, maxHeight: "85vh",
        display: "flex", flexDirection: "column",
      }}>
        {/* Header */}
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 20, flexShrink: 0 }}>
          <h2 style={{ ...typography.pageTitle, color: colors.text.primary }}>Narrator Help</h2>
          <button onClick={onClose} style={{
            width: 28, height: 28, borderRadius: 6, border: "none",
            background: colors.bg.hover, color: colors.text.muted,
            cursor: "pointer", fontSize: 16,
            display: "flex", alignItems: "center", justifyContent: "center",
          }}>&times;</button>
        </div>

        {/* Scrollable content */}
        <div style={{ overflowY: "auto", flex: 1, paddingRight: 4 }}>
          {sections.map((section) => (
            <div key={section.title} style={{ marginBottom: 20 }}>
              <h3 style={{ ...typography.sectionLabel, color: colors.accent.primary, marginBottom: 10 }}>
                {section.title}
              </h3>
              <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
                {section.items.map((item) => {
                  const isOpen = expanded === item.q;
                  return (
                    <div key={item.q} style={{
                      borderRadius: 8,
                      border: `1px solid ${isOpen ? colors.border.focus : colors.border.default}`,
                      background: isOpen ? "rgba(129,140,248,0.04)" : colors.bg.input,
                      overflow: "hidden", transition: "all 0.15s ease",
                    }}>
                      <button onClick={() => toggle(item.q)} style={{
                        width: "100%", padding: "10px 14px", border: "none",
                        background: "transparent",
                        color: isOpen ? colors.text.primary : colors.text.secondary,
                        fontSize: 13, fontWeight: 600, fontFamily: "inherit",
                        textAlign: "left", cursor: "pointer",
                        display: "flex", justifyContent: "space-between", alignItems: "center",
                      }}>
                        {item.q}
                        <span style={{
                          fontSize: 11, color: colors.text.muted,
                          transition: "transform 0.15s ease",
                          transform: isOpen ? "rotate(90deg)" : "rotate(0deg)",
                          flexShrink: 0, marginLeft: 8,
                        }}>&#x25B6;</span>
                      </button>
                      {isOpen && (
                        <div style={{
                          padding: "0 14px 12px", fontSize: 13,
                          lineHeight: 1.6, color: colors.text.secondary,
                          whiteSpace: "pre-line",
                        }}>
                          {item.a}
                          {"action" in item && item.action === "check_ffmpeg" && (
                            <div style={{ marginTop: 10 }}>
                              <button onClick={handleCheckFfmpeg} style={{
                                padding: "5px 12px", borderRadius: 6, fontSize: 12,
                                fontWeight: 600, fontFamily: "inherit", cursor: "pointer",
                                border: `1px solid ${colors.border.default}`,
                                background: colors.bg.hover, color: colors.text.primary,
                              }}>Check FFmpeg Status</button>
                              {ffmpegStatus && (
                                <span style={{
                                  marginLeft: 10, fontSize: 12, fontWeight: 500,
                                  color: ffmpegStatus.startsWith("Found") ? colors.accent.green : colors.accent.red,
                                }}>{ffmpegStatus}</span>
                              )}
                            </div>
                          )}
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>
          ))}
        </div>

        {/* Send Feedback */}
        <div style={{ marginTop: 8, flexShrink: 0 }}>
          <h3 style={{ ...typography.sectionLabel, color: colors.accent.primary, marginBottom: 10 }}>Send Feedback</h3>
          {feedbackSent ? (
            <div style={{ padding: "14px 16px", borderRadius: 8, background: "rgba(34,197,94,0.08)", border: "1px solid rgba(34,197,94,0.2)", fontSize: 13, color: "#22c55e", fontWeight: 600 }}>
              Thanks for your feedback!
            </div>
          ) : (
            <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
              <div style={{ display: "flex", gap: 6 }}>
                {(["bug", "feature", "general"] as const).map((t) => (
                  <button key={t} onClick={() => setFeedbackType(t)} style={{
                    padding: "4px 12px", borderRadius: 6, fontSize: 11, fontWeight: 600, fontFamily: "inherit", cursor: "pointer",
                    border: feedbackType === t ? "1px solid rgba(99,102,241,0.4)" : `1px solid ${colors.border.default}`,
                    background: feedbackType === t ? "rgba(99,102,241,0.1)" : colors.bg.input,
                    color: feedbackType === t ? colors.accent.primary : colors.text.muted,
                    textTransform: "capitalize",
                  }}>{t === "bug" ? "Bug Report" : t === "feature" ? "Feature Request" : "General"}</button>
                ))}
              </div>
              <textarea
                value={feedbackText}
                onChange={(e) => setFeedbackText(e.target.value)}
                placeholder="Describe your feedback, issue, or idea..."
                rows={3}
                style={{
                  width: "100%", padding: "10px 12px", borderRadius: 8, fontSize: 13,
                  background: colors.bg.input, border: `1px solid ${colors.border.default}`,
                  color: colors.text.primary, outline: "none", fontFamily: "inherit",
                  resize: "none", lineHeight: 1.5,
                }}
                onFocus={(e) => e.target.style.borderColor = "rgba(99,102,241,0.4)"}
                onBlur={(e) => e.target.style.borderColor = colors.border.default}
                maxLength={4000}
              />
              <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                <span style={{ fontSize: 11, color: colors.text.muted }}>{feedbackText.length}/4000</span>
                <Button
                  variant="primary"
                  size="sm"
                  disabled={!feedbackText.trim()}
                  onClick={() => {
                    trackEvent("feedback", {
                      type: feedbackType,
                      message: feedbackText.trim().slice(0, 4000),
                      os: navigator.platform,
                      screen_width: window.screen.width,
                      screen_height: window.screen.height,
                    });
                    setFeedbackText("");
                    setFeedbackSent(true);
                    setTimeout(() => setFeedbackSent(false), 5000);
                  }}
                >Send Feedback</Button>
              </div>
            </div>
          )}
        </div>

        {/* Legal */}
        {(onShowPrivacyPolicy || onShowTerms) && (
          <div style={{ marginTop: 8, flexShrink: 0 }}>
            <h3 style={{ ...typography.sectionLabel, color: colors.accent.primary, marginBottom: 10 }}>Legal</h3>
            <div style={{ display: "flex", gap: 16 }}>
              {onShowPrivacyPolicy && (
                <button onClick={onShowPrivacyPolicy} style={{
                  background: "none", border: "none", color: colors.text.secondary, fontSize: 13,
                  cursor: "pointer", fontFamily: "inherit", textDecoration: "underline", textUnderlineOffset: 2,
                }}>Privacy Policy</button>
              )}
              {onShowTerms && (
                <button onClick={onShowTerms} style={{
                  background: "none", border: "none", color: colors.text.secondary, fontSize: 13,
                  cursor: "pointer", fontFamily: "inherit", textDecoration: "underline", textUnderlineOffset: 2,
                }}>Terms of Service</button>
              )}
            </div>
          </div>
        )}

        {/* Footer */}
        <div style={{ marginTop: 16, display: "flex", justifyContent: "flex-end", flexShrink: 0 }}>
          <Button variant="secondary" onClick={onClose}>Done</Button>
        </div>
      </div>
    </div>
  );
}
