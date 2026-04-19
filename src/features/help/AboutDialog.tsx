import { useEffect, useCallback, useState } from "react";
import { colors, typography } from "../../lib/theme";
import { Button } from "../../components/ui/Button";
import { DISPLAY_VERSION, APP_BUILD_TIME, APP_GIT_SHA, formatBuildTime } from "../../lib/version";

const year = new Date().getFullYear();
const platform = navigator.platform;
const isMac = platform.toUpperCase().includes("MAC");
const platformLabel = isMac ? "macOS" : platform.includes("Win") ? "Windows" : "Linux";
const archLabel = navigator.userAgent.includes("x64") || navigator.userAgent.includes("Win64") ? "x86_64" : navigator.userAgent.includes("arm") ? "ARM64" : "x86_64";

export function AboutDialog({ onClose }: { onClose: () => void }) {
  const [copied, setCopied] = useState(false);

  const handleKeyDown = useCallback((e: KeyboardEvent) => {
    if (e.key === "Escape") onClose();
  }, [onClose]);

  useEffect(() => {
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [handleKeyDown]);

  const handleCopy = useCallback(async () => {
    const info = [
      `Version: v${DISPLAY_VERSION}`,
      `Built: ${formatBuildTime(APP_BUILD_TIME)}`,
      `Commit: ${APP_GIT_SHA}`,
      `Platform: ${platformLabel}`,
      `Architecture: ${archLabel}`,
    ].join("\n");
    try {
      await navigator.clipboard.writeText(info);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // ignore — clipboard may be blocked
    }
  }, []);

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
          padding: "14px 16px", marginBottom: 12, textAlign: "left",
        }}>
          <InfoRow label="Version" value={`v${DISPLAY_VERSION}`} />
          <InfoRow label="Built" value={formatBuildTime(APP_BUILD_TIME)} />
          <InfoRow label="Commit" value={APP_GIT_SHA} />
          <InfoRow label="Platform" value={platformLabel} />
          <InfoRow label="Architecture" value={archLabel} />
          <InfoRow label="Engine" value={isMac ? "WebKit" : "WebView2 (Chromium)"} />
          <InfoRow label="Framework" value="Tauri v2" last />
        </div>

        <div style={{ display: "flex", justifyContent: "flex-end", marginBottom: 16 }}>
          <button
            onClick={handleCopy}
            style={{
              fontSize: 11, color: copied ? "#4ade80" : colors.text.muted,
              background: "transparent", border: "none", cursor: "pointer",
              padding: "4px 8px", fontFamily: "inherit",
            }}
          >
            {copied ? "Copied!" : "Copy info"}
          </button>
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
