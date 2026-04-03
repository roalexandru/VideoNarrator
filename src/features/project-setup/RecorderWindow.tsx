import { useState, useEffect, type MouseEventHandler } from "react";
import { listScreens, listWindows, startRecording, stopRecording, closeRecorderWindow,
  type ScreenDevice, type WindowInfo, type RecordingConfig } from "../../lib/tauri/commands";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { emit } from "@tauri-apps/api/event";

type Mode = "screen" | "window" | "region";

const PRESETS = [
  { label: "Full Screen", w: 0, h: 0 },
  { label: "1080p", w: 1920, h: 1080 },
  { label: "720p", w: 1280, h: 720 },
  { label: "4K", w: 3840, h: 2160 },
];

const ScreenIcon = () => <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6"><rect x="2" y="3" width="20" height="14" rx="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/></svg>;
const WindowIcon = () => <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6"><rect x="3" y="3" width="18" height="18" rx="2"/><line x1="3" y1="9" x2="21" y2="9"/><circle cx="6.5" cy="6" r="0.8" fill="currentColor"/><circle cx="9" cy="6" r="0.8" fill="currentColor"/></svg>;
const RegionIcon = () => <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeDasharray="3 3"><rect x="4" y="4" width="16" height="16" rx="1"/></svg>;

const handleDrag: MouseEventHandler = (e) => {
  if (e.button !== 0) return;
  const t = (e.target as HTMLElement).tagName;
  if (["BUTTON", "SELECT", "INPUT"].includes(t)) return;
  e.preventDefault();
  getCurrentWindow().startDragging();
};

export function RecorderWindow() {
  const [mode, setMode] = useState<Mode>("screen");
  const [screens, setScreens] = useState<ScreenDevice[]>([]);
  const [windows, setWindows] = useState<WindowInfo[]>([]);
  const [selScreen, setSelScreen] = useState(1);
  const [selWindow, setSelWindow] = useState("");
  const [preset, setPreset] = useState(0);
  const [fps, setFps] = useState(30);
  const [recording, setRecording] = useState(false);
  const [seconds, setSeconds] = useState(0);

  useEffect(() => {
    listScreens().then((s) => { setScreens(s); if (s.length > 0) setSelScreen(s[0].index); }).catch(() => {});
    listWindows().then(setWindows).catch(() => {});
  }, []);

  useEffect(() => {
    if (!recording) return;
    const t = setInterval(() => setSeconds((s) => s + 1), 1000);
    return () => clearInterval(t);
  }, [recording]);

  const handleRecord = async () => {
    const p = PRESETS[preset];
    const config: RecordingConfig = {
      output_path: `/tmp/narrator_recording.mp4`,
      screen_index: selScreen, width: p.w, height: p.h, fps,
      offset_x: 0, offset_y: 0, capture_audio: false,
    };
    setRecording(true); setSeconds(0);
    // Tell main window to minimize
    await emit("recorder-started", {});
    try { await startRecording(config); } catch (e) { console.error(e); }
  };

  const handleStop = async () => {
    try {
      await stopRecording();
      setRecording(false);
      await new Promise((r) => setTimeout(r, 1000));
      // Tell main window to load the video and restore
      await emit("recorder-stopped", { path: "/tmp/narrator_recording.mp4" });
      await closeRecorderWindow();
    } catch (e) { console.error(e); }
  };

  const handleCancel = async () => {
    if (recording) await stopRecording();
    await closeRecorderWindow();
  };

  const fmt = (s: number) => `${Math.floor(s / 60)}:${(s % 60).toString().padStart(2, "0")}`;

  if (recording) {
    return (
      <div onMouseDown={handleDrag} style={{
        height: "100%", background: "rgba(20,20,28,0.95)", borderRadius: 14,
        display: "flex", alignItems: "center", justifyContent: "center", gap: 16,
        padding: "0 24px", cursor: "grab",
      }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <div style={{ width: 12, height: 12, borderRadius: "50%", background: "#ef4444", animation: "rp 1s infinite" }} />
          <span style={{ fontFamily: "monospace", fontSize: 22, fontWeight: 700, color: "#ef4444" }}>{fmt(seconds)}</span>
        </div>
        <button onClick={handleStop} style={{
          padding: "8px 20px", borderRadius: 8, border: "none",
          background: "#ef4444", color: "#fff", fontSize: 13, fontWeight: 600,
          cursor: "pointer", fontFamily: "inherit",
        }}>
          <svg width="10" height="10" viewBox="0 0 24 24" fill="currentColor" style={{ marginRight: 6, verticalAlign: "middle" }}><rect x="4" y="4" width="16" height="16" rx="2"/></svg>
          Stop
        </button>
        <style>{`@keyframes rp { 0%,100% { opacity:1; } 50% { opacity:0.3; } }`}</style>
      </div>
    );
  }

  const sel = { padding: "6px 10px", borderRadius: 7, border: "1px solid rgba(255,255,255,0.08)", background: "rgba(255,255,255,0.05)", color: "#c8c8d8", fontSize: 12, fontFamily: "inherit", appearance: "none" as const, outline: "none", cursor: "pointer" } as const;

  return (
    <div onMouseDown={handleDrag} style={{
      height: "100%", background: "rgba(20,20,28,0.95)", borderRadius: 14,
      padding: "20px 24px", display: "flex", flexDirection: "column",
      cursor: "grab",
    }}>
      {/* Mode selector */}
      <div style={{ display: "flex", gap: 6, marginBottom: 16 }}>
        {([
          { m: "screen" as const, icon: <ScreenIcon />, label: "Screen" },
          { m: "window" as const, icon: <WindowIcon />, label: "Window" },
          { m: "region" as const, icon: <RegionIcon />, label: "Region" },
        ]).map(({ m, icon, label }) => (
          <button key={m} onClick={() => setMode(m)} style={{
            flex: 1, padding: "10px 6px", borderRadius: 8, border: "none",
            background: mode === m ? "rgba(99,102,241,0.18)" : "rgba(255,255,255,0.03)",
            color: mode === m ? "#a5b4fc" : "#5a5a6e",
            cursor: "pointer", display: "flex", flexDirection: "column", alignItems: "center", gap: 4,
            fontSize: 11, fontWeight: 600, fontFamily: "inherit",
            outline: mode === m ? "1px solid rgba(99,102,241,0.25)" : "1px solid rgba(255,255,255,0.05)",
          }}>{icon}{label}</button>
        ))}
      </div>

      {/* Target */}
      <div style={{ marginBottom: 12 }}>
        {mode === "screen" && (
          <div style={{ display: "flex", gap: 6 }}>
            <select value={selScreen} onChange={(e) => setSelScreen(parseInt(e.target.value))} style={{ ...sel, flex: 1 }}>
              {screens.map((s) => <option key={s.index} value={s.index} style={{ background: "#14141c" }}>{s.name}</option>)}
            </select>
            <select value={preset} onChange={(e) => setPreset(parseInt(e.target.value))} style={{ ...sel, width: 100 }}>
              {PRESETS.map((p, i) => <option key={i} value={i} style={{ background: "#14141c" }}>{p.label}</option>)}
            </select>
          </div>
        )}
        {mode === "window" && (
          <select value={selWindow} onChange={(e) => setSelWindow(e.target.value)} style={{ ...sel, width: "100%" }}>
            <option value="" style={{ background: "#14141c" }}>Choose window...</option>
            {windows.map((w, i) => <option key={i} value={w.owner} style={{ background: "#14141c" }}>{w.owner}{w.name ? ` — ${w.name.slice(0, 30)}` : ""}</option>)}
          </select>
        )}
        {mode === "region" && <p style={{ fontSize: 12, color: "#5a5a6e", textAlign: "center" }}>Full screen captured — trim in Edit Video</p>}
      </div>

      {/* FPS */}
      <div style={{ display: "flex", gap: 4, marginBottom: 16, alignItems: "center" }}>
        <span style={{ fontSize: 11, color: "#5a5a6e", marginRight: 4 }}>FPS</span>
        {[15, 30, 60].map((f) => (
          <button key={f} onClick={() => setFps(f)} style={{
            padding: "4px 10px", borderRadius: 5, border: "none",
            background: fps === f ? "rgba(99,102,241,0.15)" : "rgba(255,255,255,0.03)",
            color: fps === f ? "#a5b4fc" : "#5a5a6e",
            fontSize: 11, fontWeight: 600, cursor: "pointer", fontFamily: "inherit",
          }}>{f}</button>
        ))}
      </div>

      {/* Actions */}
      <div style={{ display: "flex", gap: 8, marginTop: "auto" }}>
        <button onClick={handleCancel} style={{
          flex: 1, padding: "8px", borderRadius: 8, border: "1px solid rgba(255,255,255,0.06)",
          background: "transparent", color: "#6b6b80", fontSize: 13, cursor: "pointer", fontFamily: "inherit",
        }}>Cancel</button>
        <button onClick={handleRecord} style={{
          flex: 2, padding: "8px", borderRadius: 8, border: "none",
          background: "#ef4444", color: "#fff", fontSize: 13, fontWeight: 600,
          cursor: "pointer", fontFamily: "inherit",
          display: "flex", alignItems: "center", justifyContent: "center", gap: 6,
        }}>
          <div style={{ width: 8, height: 8, borderRadius: "50%", background: "#fff" }} />
          Record
        </button>
      </div>
    </div>
  );
}
