import { useEffect, useCallback } from "react";
import { colors, typography } from "../../lib/theme";
import { Button } from "../../components/ui/Button";

const year = new Date().getFullYear();
const platform = navigator.platform;
const isMac = platform.toUpperCase().includes("MAC");

export function AboutDialog({ onClose }: { onClose: () => void }) {
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
      aria-labelledby="about-title"
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
        padding: "32px 36px", width: 400, textAlign: "center",
      }}>
        {/* Logo + Name */}
        <div style={{
          width: 56, height: 56, borderRadius: 14, margin: "0 auto 16px",
          background: "linear-gradient(135deg, #6366f1, #8b5cf6)",
          display: "flex", alignItems: "center", justifyContent: "center",
        }}>
          <svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="#fff" strokeWidth="2" strokeLinecap="round">
            <polygon points="5 3 19 12 5 21 5 3" />
          </svg>
        </div>
        <h2 id="about-title" style={{ ...typography.pageTitle, color: colors.text.primary, marginBottom: 4 }}>Narrator</h2>
        <div style={{ fontSize: 14, color: colors.text.secondary, marginBottom: 20 }}>
          AI-Powered Video Narration
        </div>

        {/* Version info */}
        <div style={{
          background: colors.bg.input, borderRadius: 10,
          border: `1px solid ${colors.border.default}`,
          padding: "14px 16px", marginBottom: 20, textAlign: "left",
        }}>
          <InfoRow label="Version" value={`v${__APP_VERSION__}`} />
          <InfoRow label="Platform" value={isMac ? "macOS" : platform.includes("Win") ? "Windows" : "Linux"} />
          <InfoRow label="Architecture" value={navigator.userAgent.includes("x64") || navigator.userAgent.includes("Win64") ? "x86_64" : navigator.userAgent.includes("arm") ? "ARM64" : "x86_64"} />
          <InfoRow label="Engine" value={isMac ? "WebKit" : "WebView2 (Chromium)"} />
          <InfoRow label="Framework" value="Tauri v2" last />
        </div>

        {/* Credits */}
        <div style={{ fontSize: 11, color: colors.text.muted, lineHeight: 1.6, marginBottom: 20 }}>
          Built with Tauri, React, and Rust.
          <br />
          Video processing powered by FFmpeg.
        </div>

        {/* Copyright */}
        <div style={{ fontSize: 11, color: colors.text.muted, marginBottom: 20 }}>
          &copy; {year} Narrator. All rights reserved.
        </div>

        <Button variant="secondary" onClick={onClose}>Close</Button>
      </div>
    </div>
  );
}

function InfoRow({ label, value, last }: { label: string; value: string; last?: boolean }) {
  return (
    <div style={{
      display: "flex", justifyContent: "space-between", alignItems: "center",
      padding: "6px 0",
      borderBottom: last ? "none" : `1px solid ${colors.border.default}`,
    }}>
      <span style={{ fontSize: 12, color: colors.text.muted }}>{label}</span>
      <span style={{ fontSize: 12, color: colors.text.primary, fontWeight: 500 }}>{value}</span>
    </div>
  );
}
