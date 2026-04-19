import { useState, useEffect, useCallback } from "react";
import { colors, typography } from "../../lib/theme";
import { Button } from "../../components/ui/Button";
import { checkFfmpeg } from "../../lib/tauri/commands";

const isMac = navigator.platform.toUpperCase().includes("MAC");

const sections = [
  {
    title: "Getting Started",
    items: [
      {
        q: "What is Narrator?",
        a: "Narrator is an AI-powered desktop app that generates narration scripts from videos. Import a video or record your screen, and the AI analyzes visual content frame-by-frame to produce a professional narration. Export as video with voice-over, audio-only, or subtitle files.",
      },
      {
        q: "Prerequisites",
        a: "• FFmpeg — required for video processing (brew install ffmpeg on macOS, choco install ffmpeg on Windows)\n• At least one AI provider API key: Anthropic Claude, OpenAI, or Google Gemini\n• For text-to-speech export: ElevenLabs or Azure TTS API key (or use the free Built-in voice)",
      },
      {
        q: "Quick start",
        a: `1. Open Settings (${isMac ? "⌘," : "Ctrl+,"}) and add your AI provider API key.\n2. Click New Project — import a video or record your screen.\n3. Optionally add context documents (PDF, Markdown, TXT) to improve narration quality.\n4. Choose a narration style, language, and detail level in Configuration.\n5. Click Start Generation in the Processing step.\n6. Review, edit, and refine segments — then export.`,
      },
    ],
  },
  {
    title: "Project Setup",
    items: [
      {
        q: "Importing video",
        a: "Click \"Select Video File\" to browse for MP4, MOV, AVI, MKV, or WebM files. You can also drag and drop video files directly onto the Setup screen.",
      },
      {
        q: "Screen recording",
        a: "Click \"Record Screen\" to capture your screen directly. On macOS, this uses the native screen capture. On Windows, a recording overlay with start/stop/pause controls appears.",
      },
      {
        q: "Context documents",
        a: "Attach Markdown, TXT, or PDF files to give the AI background about your content — brand guides, product docs, or glossaries. These improve narration accuracy. Drag and drop or click \"+ Add\".",
      },
      {
        q: "Project sharing",
        a: "Share a project as a portable .narrator file (hover a project card, click \"Share\"). This includes config, scripts, and frames. Import on any machine (Mac or Windows) via \"Import .narrator\" on the Projects screen. The video file is not bundled — re-link it after import.",
      },
    ],
  },
  {
    title: "Edit Video",
    items: [
      {
        q: "Trimming and splitting",
        a: "Use the timeline to select portions of your video. Press S to split at the playhead. Delete unwanted clips. Adjust speed (0.25x–10x) or enable time-lapse mode for clean jump-cuts.",
      },
      {
        q: "Freeze frame",
        a: "Hold a single still frame for a fixed duration — handy for emphasizing a state before the video continues. Pick a source time and a duration in the clip panel; freeze clips add to output length without consuming source audio.",
      },
      {
        q: "Effects overview",
        a: "Add effects on the FX track below the timeline. Five types: Zoom (animate between two regions of the frame), Spotlight (dim everything except a circular area), Blur (Gaussian blur of a rectangular region, or the inverse — blur everything except the rect), Text (overlay text with font/colour/background), and Fade (blend a solid colour over the whole frame). Effects compose on top of speed-changed sections and multi-clip cuts — they time on the output timeline, not the source.",
      },
      {
        q: "Effect transitions (Zoom In / Hold / Zoom Out)",
        a: "Every effect has transitionIn + transitionOut values (in seconds) plus a Return toggle. With Return on, the effect ramps 0→1 over transitionIn, holds at full strength for the middle, then ramps 1→0 over transitionOut. Without Return, it ramps in and stays at full until the effect ends. The Smooth/Linear/Ease In/Ease Out preset controls the ramp curve.",
      },
      {
        q: "Stacking effects",
        a: "Effects composite in declaration order. Zoom OVERWRITES the frame with the cropped-and-scaled region, so put it FIRST in a stack — Spotlight / Blur / Text / Fade drawn afterwards will appear on top of the zoomed frame. Blur / Spotlight / Text / Fade fade in smoothly with the configured transition; on a zoomed section they operate in screen space (e.g. a blur rect in the top-right stays in the top-right regardless of zoom).",
      },
      {
        q: "Per-clip zoom vs overlay zoom",
        a: "Per-clip zoom lives on a clip and ramps linearly across the whole clip output duration (no transition controls). Overlay Zoom on the FX track is the full animation model — start region, end region, transitionIn/Hold/Out, reverse, easing. Prefer the overlay version for anything beyond a simple whole-clip pan.",
      },
      {
        q: "Controls",
        a: `Play/Pause — Space bar\nSplit at playhead — S\nUndo — ${isMac ? "⌘Z" : "Ctrl+Z"}\nRedo — ${isMac ? "⌘⇧Z" : "Ctrl+Shift+Z"}\nZoom timeline — ${isMac ? "⌘" : "Ctrl"}+Scroll`,
      },
    ],
  },
  {
    title: "Configuration",
    items: [
      {
        q: "Narration styles",
        a: "Choose from 6 styles: Executive Overview, Product Demo, Technical Deep-Dive, Teaser/Trailer, Training Walkthrough, and Bug Review/Critique. Each style tunes the AI's tone, vocabulary, and focus.",
      },
      {
        q: "Languages",
        a: "Select one or more languages. The primary language is generated from video frames; additional languages are translated from it. You can also translate after generation via the \"+ Translate\" button in Review.",
      },
      {
        q: "Detail level",
        a: "Light — key points only, fewer frames extracted. Medium — balanced coverage. Heavy — detailed commentary with more frames for better visual analysis.",
      },
      {
        q: "Templates",
        a: "Save your current configuration as a reusable template in Settings > Templates. Apply a template to any project to instantly set style, language, AI provider, and density.",
      },
    ],
  },
  {
    title: "Review",
    items: [
      {
        q: "Editing segments",
        a: `Each segment shows a timestamp, narration text (editable), pace, and action buttons. Click the timestamp to fine-tune start/end times. Use ${isMac ? "⌘Z" : "Ctrl+Z"} to undo and ${isMac ? "⌘⇧Z" : "Ctrl+Shift+Z"} to redo any script edit.`,
      },
      {
        q: "AI refinement",
        a: "Click the \"AI\" button on any segment to refine it with AI. Choose a preset (Make shorter, More detailed, Simplify, More professional, More conversational) or type a custom instruction. The AI rewrites that segment while considering surrounding context.",
      },
      {
        q: "Preview narration",
        a: "Click \"Preview Narration\" to hear the full narrated video — TTS audio plays synced with the video and subtitles appear live. Audio is cached per-segment for instant replays. Click Stop Preview to cancel at any time.",
      },
      {
        q: "Per-segment voice",
        a: "Click the \"Voice\" badge on any segment to assign a different TTS voice from your configured provider. \"Project default\" uses the global voice from Settings > Voice.",
      },
      {
        q: "Multi-language",
        a: "When multiple languages exist, tabs appear in the header. Switch between languages to edit each independently. Use \"+ Translate\" to generate a new language from the current script.",
      },
    ],
  },
  {
    title: "Export",
    items: [
      {
        q: "Video export",
        a: "Generate a video with narration audio. Choose \"Narration only\" to replace original audio, or \"Mix with original\" to overlay. Enable \"Burn subtitles\" for embedded captions — customize font size, color, outline, and position.",
      },
      {
        q: "Audio export",
        a: "Generate narration audio without video. Uses the voice settings from Settings > Voice.",
      },
      {
        q: "Script export",
        a: "Export narration text as SRT (subtitles), VTT (web video), JSON (programmatic), Markdown (readable), or SSML (speech synthesis). Select formats and languages before exporting.",
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
        q: "Review",
        a: isMac
          ? "Undo — ⌘Z\nRedo — ⌘⇧Z\nPlay/Pause video — Space"
          : "Undo — Ctrl+Z\nRedo — Ctrl+Shift+Z\nPlay/Pause video — Space",
      },
      {
        q: "Edit Video",
        a: `Split at playhead — S\nPlay/Pause — Space\nUndo — ${isMac ? "⌘Z" : "Ctrl+Z"}\nRedo — ${isMac ? "⌘⇧Z" : "Ctrl+Shift+Z"}\nZoom — ${isMac ? "⌘" : "Ctrl"}+Scroll`,
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
        a: `Open Settings (${isMac ? "⌘," : "Ctrl+,"}) and verify your API key is correct. Keys are validated when saved. Ensure billing is enabled on your provider account with sufficient quota.`,
      },
      {
        q: "Generation fails or poor results",
        a: "Try a different narration style or lower the temperature (0.3–0.5) for consistency. Adding context documents helps the AI understand domain-specific content. For long videos, use \"Heavy\" density to capture more visual detail. Note: OpenAI reasoning models (o1, o3, o4, GPT-5) don't support a custom temperature — the slider auto-disables when you pick one and the model runs at its default.",
      },
      {
        q: "Export is slow",
        a: "Export runs a frame-by-frame Rust compositor — a 4-minute 1080p video with effects typically renders in a few minutes on Apple Silicon. Development builds (pnpm tauri dev) are 10–20× slower than release builds; always use a release/installed build for real renders. Expected throughput is roughly 1× realtime at 1080p30 for multi-effect timelines.",
      },
      {
        q: "Audio/video merge issues",
        a: "Ensure FFmpeg is up to date (version 5+ recommended). If segments are missing audio, regenerate TTS for the affected segments. Check that your TTS API key has sufficient quota.",
      },
    ],
  },
];

export function HelpPanel({ onClose, onShowPrivacyPolicy, onShowTerms }: { onClose: () => void; onShowPrivacyPolicy?: () => void; onShowTerms?: () => void }) {
  const [expanded, setExpanded] = useState<string | null>(sections[0].items[0].q);
  const [ffmpegStatus, setFfmpegStatus] = useState<string | null>(null);

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
