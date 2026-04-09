import { useState, useEffect, useCallback, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { trackError } from "../telemetry/analytics";

export function RecorderOverlay() {
  const [initializing, setInitializing] = useState(true);
  const [paused, setPaused] = useState(false);
  const [seconds, setSeconds] = useState(0);
  const [busy, setBusy] = useState(false);

  // Listen for the backend "recording-started" event (FFmpeg is ready)
  useEffect(() => {
    const unlisten = listen("recording-started", () => setInitializing(false));
    return () => { unlisten.then(fn => fn()); };
  }, []);

  // Timer — counts up while recording, freezes while paused or initializing
  useEffect(() => {
    if (initializing || paused) return;
    const t = setInterval(() => setSeconds((s) => s + 1), 1000);
    return () => clearInterval(t);
  }, [initializing, paused]);

  const handlePause = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    try {
      await invoke("pause_recording");
      setPaused(true);
    } catch (e) {
      console.error("Pause failed:", e);
      trackError("recording_pause", e);
    }
    setBusy(false);
  }, [busy]);

  const handleResume = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    try {
      await invoke("resume_recording");
      setPaused(false);
    } catch (e) {
      console.error("Resume failed:", e);
      trackError("recording_resume", e);
    }
    setBusy(false);
  }, [busy]);

  const handleStop = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    try {
      await invoke("stop_screen_recording");
      // Overlay will be closed by the backend
    } catch (e) {
      console.error("Stop failed:", e);
      trackError("recording_stop", e);
      setBusy(false);
    }
  }, [busy]);

  const mm = Math.floor(seconds / 60);
  const ss = (seconds % 60).toString().padStart(2, "0");

  if (initializing) {
    return (
      <div style={container}>
        <div data-tauri-drag-region style={dragBar} onDoubleClick={() => {}} />
        <div style={content}>
          <span style={initText}>Starting...</span>
        </div>
      </div>
    );
  }

  return (
    <div style={container}>
      {/* Drag region */}
      <div data-tauri-drag-region style={dragBar} onDoubleClick={() => {}} />

      <div style={content}>
        {/* Recording indicator + timer */}
        <div style={timerSection}>
          <div style={{ ...dot, animation: paused ? "none" : "recpulse 1s infinite" }} />
          <span style={timerText}>{mm}:{ss}</span>
          {paused && <span style={pausedLabel}>Paused</span>}
        </div>

        {/* Controls */}
        <div style={controls}>
          {paused ? (
            <button onClick={handleResume} disabled={busy} style={btnResume} title="Resume">
              <svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor">
                <path d="M4 2l10 6-10 6V2z" />
              </svg>
            </button>
          ) : (
            <button onClick={handlePause} disabled={busy} style={btnPause} title="Pause">
              <svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor">
                <rect x="3" y="2" width="4" height="12" rx="1" />
                <rect x="9" y="2" width="4" height="12" rx="1" />
              </svg>
            </button>
          )}
          <button onClick={handleStop} disabled={busy} style={btnStop} title="Stop">
            <svg width="12" height="12" viewBox="0 0 16 16" fill="currentColor">
              <rect x="2" y="2" width="12" height="12" rx="2" />
            </svg>
          </button>
        </div>
      </div>

      <style>{`
        @keyframes recpulse { 0%,100% { opacity:1; } 50% { opacity:0.3; } }
        button:hover:not(:disabled) { filter: brightness(1.2); }
        button:disabled { opacity: 0.5; cursor: default !important; }
      `}</style>
    </div>
  );
}

// ── Styles ──

const container: CSSProperties = {
  width: "100%",
  height: "100%",
  background: "#0f0f14",
  overflow: "hidden",
  fontFamily:
    '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif',
};

const dragBar: CSSProperties = {
  height: 14,
  cursor: "move",
};

const content: CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  padding: "0 16px 0",
  height: "calc(100% - 14px)",
};

const timerSection: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
};

const dot: CSSProperties = {
  width: 10,
  height: 10,
  borderRadius: "50%",
  background: "#ef4444",
  flexShrink: 0,
};

const initText: CSSProperties = {
  fontSize: 14,
  fontWeight: 600,
  color: "#a0a0b0",
  letterSpacing: 0.5,
};

const timerText: CSSProperties = {
  fontFamily: "monospace",
  fontSize: 18,
  fontWeight: 700,
  color: "#e0e0ea",
  letterSpacing: 1,
};

const pausedLabel: CSSProperties = {
  fontSize: 10,
  fontWeight: 600,
  color: "#fbbf24",
  textTransform: "uppercase",
  letterSpacing: 0.8,
};

const controls: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
};

const btnBase: CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  width: 34,
  height: 34,
  borderRadius: 10,
  border: "none",
  cursor: "pointer",
  transition: "filter 0.15s",
};

const btnPause: CSSProperties = {
  ...btnBase,
  background: "rgba(255,255,255,0.1)",
  color: "#e0e0ea",
};

const btnResume: CSSProperties = {
  ...btnBase,
  background: "rgba(34,197,94,0.2)",
  color: "#22c55e",
};

const btnStop: CSSProperties = {
  ...btnBase,
  background: "rgba(239,68,68,0.2)",
  color: "#ef4444",
};
