import { useState, useEffect } from "react";
import { listScreens, listWindows, startRecording, stopRecording, probeVideo,
  type ScreenDevice, type WindowInfo, type RecordingConfig } from "../../lib/tauri/commands";
import { useProjectStore } from "../../stores/projectStore";
import { getCurrentWindow } from "@tauri-apps/api/window";

type Mode = "screen" | "window" | "region";
type Phase = "setup" | "countdown" | "recording";

const PRESETS = [
  { label: "Full Screen", w: 0, h: 0 },
  { label: "1080p", w: 1920, h: 1080 },
  { label: "720p", w: 1280, h: 720 },
  { label: "4K", w: 3840, h: 2160 },
];

// SVG icons as components for clarity
const ScreenIcon = () => <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round"><rect x="2" y="3" width="20" height="14" rx="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/></svg>;
const WindowIcon = () => <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round"><rect x="3" y="3" width="18" height="18" rx="2"/><line x1="3" y1="9" x2="21" y2="9"/><circle cx="6.5" cy="6" r="1" fill="currentColor"/><circle cx="9.5" cy="6" r="1" fill="currentColor"/></svg>;
const RegionIcon = () => <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeDasharray="3 3"><rect x="4" y="4" width="16" height="16" rx="1"/></svg>;

export function RecorderToolbar({ onClose, onDone }: { onClose: () => void; onDone: () => void }) {
  const projectId = useProjectStore((s) => s.projectId);
  const setVideoFile = useProjectStore((s) => s.setVideoFile);

  const [phase, setPhase] = useState<Phase>("setup");
  const [mode, setMode] = useState<Mode>("screen");
  const [screens, setScreens] = useState<ScreenDevice[]>([]);
  const [windows, setWindows] = useState<WindowInfo[]>([]);
  const [selScreen, setSelScreen] = useState(1);
  const [selWindow, setSelWindow] = useState("");
  const [preset, setPreset] = useState(0);
  const [fps, setFps] = useState(30);
  const [seconds, setSeconds] = useState(0);
  const [countNum, setCountNum] = useState(3);

  useEffect(() => {
    listScreens().then((s) => { setScreens(s); if (s.length > 0) setSelScreen(s[0].index); }).catch(() => {});
    listWindows().then(setWindows).catch(() => {});
  }, []);

  useEffect(() => {
    if (phase !== "recording") return;
    const t = setInterval(() => setSeconds((s) => s + 1), 1000);
    return () => clearInterval(t);
  }, [phase]);

  // Esc to stop while recording
  useEffect(() => {
    if (phase !== "recording") return;
    const h = (e: KeyboardEvent) => { if (e.code === "Escape") { e.preventDefault(); handleStop(); } };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [phase]);

  const handleRecord = async () => {
    setPhase("countdown");
    for (let i = 3; i > 0; i--) { setCountNum(i); await new Promise((r) => setTimeout(r, 1000)); }

    // Minimize app so it's not captured
    try { await getCurrentWindow().minimize(); } catch {}

    const p = PRESETS[preset];
    const config: RecordingConfig = {
      output_path: `/tmp/narrator_recording_${projectId}.mp4`,
      screen_index: selScreen, width: p.w, height: p.h, fps,
      offset_x: 0, offset_y: 0, capture_audio: false,
    };
    setPhase("recording"); setSeconds(0);
    try { await startRecording(config); } catch (e) { console.error(e); }
  };

  const handleStop = async () => {
    try {
      await stopRecording(); setPhase("setup");
      try { await getCurrentWindow().unminimize(); await getCurrentWindow().setFocus(); } catch {}
      await new Promise((r) => setTimeout(r, 1500));
      try {
        const m = await probeVideo(`/tmp/narrator_recording_${projectId}.mp4`);
        setVideoFile({ path: m.path, name: "Screen Recording", size: m.file_size, duration: m.duration_seconds, resolution: { width: m.width, height: m.height }, codec: m.codec, fps: m.fps });
        onDone();
      } catch {}
    } catch {}
  };

  const fmt = (s: number) => `${Math.floor(s / 60)}:${(s % 60).toString().padStart(2, "0")}`;

  // Countdown
  if (phase === "countdown") {
    return (
      <div style={{ position: "fixed", inset: 0, zIndex: 10000, background: "rgba(0,0,0,0.85)", display: "flex", alignItems: "center", justifyContent: "center", flexDirection: "column", gap: 12 }}>
        <div style={{ fontSize: 80, fontWeight: 700, color: "#fff" }}>{countNum}</div>
        <p style={{ color: "rgba(255,255,255,0.4)", fontSize: 14 }}>App will minimize and recording starts...</p>
      </div>
    );
  }

  // Recording — this shows briefly before window minimizes, and again when user un-minimizes to stop
  if (phase === "recording") {
    return (
      <div style={{ position: "fixed", inset: 0, zIndex: 10000, background: "rgba(0,0,0,0.92)", display: "flex", alignItems: "center", justifyContent: "center", flexDirection: "column", gap: 20 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
          <div style={{ width: 14, height: 14, borderRadius: "50%", background: "#ef4444", animation: "recpulse 1s infinite" }} />
          <span style={{ fontFamily: "monospace", fontSize: 36, fontWeight: 700, color: "#ef4444" }}>{fmt(seconds)}</span>
        </div>
        <p style={{ color: "rgba(255,255,255,0.35)", fontSize: 14 }}>Recording in progress</p>
        <button onClick={handleStop} style={{
          padding: "12px 40px", borderRadius: 12, border: "none", background: "#ef4444",
          color: "#fff", fontSize: 16, fontWeight: 600, cursor: "pointer", fontFamily: "inherit",
          display: "flex", alignItems: "center", gap: 8,
        }}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor"><rect x="4" y="4" width="16" height="16" rx="2"/></svg>
          Stop Recording
        </button>
        <style>{`@keyframes recpulse { 0%,100% { opacity:1; } 50% { opacity:0.3; } }`}</style>
      </div>
    );
  }

  // ── Setup overlay — clean modal ──
  const sel = { padding: "7px 10px", borderRadius: 8, border: "1px solid rgba(255,255,255,0.08)", background: "rgba(255,255,255,0.05)", color: "#c8c8d8", fontSize: 13, fontFamily: "inherit", appearance: "none" as const, outline: "none", cursor: "pointer" } as const;

  return (
    <div style={{ position: "fixed", inset: 0, zIndex: 9999, background: "rgba(0,0,0,0.5)", backdropFilter: "blur(4px)", display: "flex", alignItems: "center", justifyContent: "center" }}
      onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div style={{
        background: "rgba(26,26,34,0.96)", borderRadius: 18,
        border: "1px solid rgba(255,255,255,0.07)",
        boxShadow: "0 24px 64px rgba(0,0,0,0.5)",
        padding: "28px 32px", width: 400,
      }}>
        <h3 style={{ fontSize: 18, fontWeight: 700, color: "#e0e0ea", marginBottom: 20, textAlign: "center" }}>Screen Recording</h3>

        {/* Mode selector — 3 big buttons */}
        <div style={{ display: "flex", gap: 8, marginBottom: 20 }}>
          {([
            { m: "screen" as const, icon: <ScreenIcon />, label: "Entire Screen" },
            { m: "window" as const, icon: <WindowIcon />, label: "Window" },
            { m: "region" as const, icon: <RegionIcon />, label: "Region" },
          ]).map(({ m, icon, label }) => (
            <button key={m} onClick={() => setMode(m)} style={{
              flex: 1, padding: "14px 8px", borderRadius: 10, border: "none",
              background: mode === m ? "rgba(99,102,241,0.15)" : "rgba(255,255,255,0.03)",
              color: mode === m ? "#a5b4fc" : "#6b6b80",
              cursor: "pointer", display: "flex", flexDirection: "column", alignItems: "center", gap: 6,
              fontSize: 11, fontWeight: 600, fontFamily: "inherit",
              transition: "all 0.12s",
              outline: mode === m ? "2px solid rgba(99,102,241,0.3)" : "1px solid rgba(255,255,255,0.06)",
            }}>
              {icon}
              {label}
            </button>
          ))}
        </div>

        {/* Target selector */}
        <div style={{ marginBottom: 16 }}>
          {mode === "screen" && (
            <div style={{ display: "flex", gap: 8 }}>
              <select value={selScreen} onChange={(e) => setSelScreen(parseInt(e.target.value))} style={{ ...sel, flex: 1 }}>
                {screens.map((s) => <option key={s.index} value={s.index} style={{ background: "#1a1a22" }}>{s.name}</option>)}
              </select>
              <select value={preset} onChange={(e) => setPreset(parseInt(e.target.value))} style={{ ...sel, width: 110 }}>
                {PRESETS.map((p, i) => <option key={i} value={i} style={{ background: "#1a1a22" }}>{p.label}</option>)}
              </select>
            </div>
          )}
          {mode === "window" && (
            <select value={selWindow} onChange={(e) => setSelWindow(e.target.value)} style={{ ...sel, width: "100%" }}>
              <option value="" style={{ background: "#1a1a22" }}>Choose a window to record...</option>
              {windows.map((w, i) => <option key={i} value={w.owner} style={{ background: "#1a1a22" }}>{w.owner}{w.name ? ` — ${w.name.slice(0, 40)}` : ""}</option>)}
            </select>
          )}
          {mode === "region" && (
            <p style={{ fontSize: 13, color: "#6b6b80", textAlign: "center" }}>Region recording: captures the full screen, trim in Edit Video after.</p>
          )}
        </div>

        {/* FPS row */}
        <div style={{ display: "flex", gap: 8, marginBottom: 24, alignItems: "center" }}>
          <span style={{ fontSize: 12, color: "#6b6b80" }}>Frame rate:</span>
          {[15, 30, 60].map((f) => (
            <button key={f} onClick={() => setFps(f)} style={{
              padding: "5px 12px", borderRadius: 6, border: "none",
              background: fps === f ? "rgba(99,102,241,0.15)" : "rgba(255,255,255,0.04)",
              color: fps === f ? "#a5b4fc" : "#6b6b80",
              fontSize: 12, fontWeight: 600, cursor: "pointer", fontFamily: "inherit",
            }}>{f} fps</button>
          ))}
        </div>

        {/* Actions */}
        <div style={{ display: "flex", gap: 10 }}>
          <button onClick={onClose} style={{
            flex: 1, padding: "10px", borderRadius: 10, border: "1px solid rgba(255,255,255,0.08)",
            background: "transparent", color: "#8b8ba0", fontSize: 14, cursor: "pointer", fontFamily: "inherit",
          }}>Cancel</button>
          <button onClick={handleRecord} style={{
            flex: 2, padding: "10px", borderRadius: 10, border: "none",
            background: "#ef4444", color: "#fff", fontSize: 14, fontWeight: 600,
            cursor: "pointer", fontFamily: "inherit",
            display: "flex", alignItems: "center", justifyContent: "center", gap: 8,
          }}>
            <div style={{ width: 10, height: 10, borderRadius: "50%", background: "#fff" }} />
            Record
          </button>
        </div>
      </div>
    </div>
  );
}
