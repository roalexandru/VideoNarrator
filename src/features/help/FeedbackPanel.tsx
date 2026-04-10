import { useState, useEffect, useCallback } from "react";
import { colors, typography } from "../../lib/theme";
import { Button } from "../../components/ui/Button";
import { trackEvent } from "../telemetry/analytics";

export function FeedbackPanel({ onClose }: { onClose: () => void }) {
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

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="feedback-title"
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
        padding: "28px 32px", width: 480,
        display: "flex", flexDirection: "column",
      }}>
        {/* Header */}
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 20 }}>
          <h2 id="feedback-title" style={{ ...typography.pageTitle, color: colors.text.primary }}>Send Feedback</h2>
          <button onClick={onClose} style={{
            width: 28, height: 28, borderRadius: 6, border: "none",
            background: colors.bg.hover, color: colors.text.muted,
            cursor: "pointer", fontSize: 16,
            display: "flex", alignItems: "center", justifyContent: "center",
          }}>&times;</button>
        </div>

        {feedbackSent ? (
          <div style={{ padding: "20px 16px", borderRadius: 8, background: "rgba(34,197,94,0.08)", border: "1px solid rgba(34,197,94,0.2)", fontSize: 14, color: "#22c55e", fontWeight: 600, textAlign: "center" }}>
            Thanks for your feedback!
          </div>
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
            <div>
              <div style={{ ...typography.sectionLabel, color: colors.text.muted, marginBottom: 8 }}>Type</div>
              <div style={{ display: "flex", gap: 6 }}>
                {(["bug", "feature", "general"] as const).map((t) => (
                  <button key={t} onClick={() => setFeedbackType(t)} style={{
                    padding: "6px 14px", borderRadius: 6, fontSize: 12, fontWeight: 600, fontFamily: "inherit", cursor: "pointer",
                    border: feedbackType === t ? "1px solid rgba(99,102,241,0.4)" : `1px solid ${colors.border.default}`,
                    background: feedbackType === t ? "rgba(99,102,241,0.1)" : colors.bg.input,
                    color: feedbackType === t ? colors.accent.primary : colors.text.muted,
                    textTransform: "capitalize",
                  }}>{t === "bug" ? "Bug Report" : t === "feature" ? "Feature Request" : "General"}</button>
                ))}
              </div>
            </div>
            <div>
              <div style={{ ...typography.sectionLabel, color: colors.text.muted, marginBottom: 8 }}>Message</div>
              <textarea
                value={feedbackText}
                onChange={(e) => setFeedbackText(e.target.value)}
                placeholder="Describe your feedback, issue, or idea..."
                rows={5}
                style={{
                  width: "100%", padding: "10px 12px", borderRadius: 8, fontSize: 13,
                  background: colors.bg.input, border: `1px solid ${colors.border.default}`,
                  color: colors.text.primary, outline: "none", fontFamily: "inherit",
                  resize: "none", lineHeight: 1.5,
                }}
                onFocus={(e) => e.target.style.borderColor = "rgba(99,102,241,0.4)"}
                onBlur={(e) => e.target.style.borderColor = colors.border.default}
                maxLength={4000}
                autoFocus
              />
            </div>
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
              <span style={{ fontSize: 11, color: colors.text.muted }}>{feedbackText.length}/4000</span>
              <div style={{ display: "flex", gap: 8 }}>
                <Button variant="secondary" size="sm" onClick={onClose}>Cancel</Button>
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
                    setTimeout(() => { setFeedbackSent(false); onClose(); }, 2000);
                  }}
                >Send Feedback</Button>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
