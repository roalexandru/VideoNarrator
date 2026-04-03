import { useState, useEffect, type CSSProperties, type MouseEventHandler, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";

type Mode = "screen" | "window" | "region";

interface ScreenDevice { index: number; name: string; is_screen: boolean; }
interface RecordingConfig {
  output_path: string; screen_index: number;
  width: number; height: number; fps: number;
  offset_x: number; offset_y: number; capture_audio: boolean;
}

const PRESETS = [
  { label: "Full Screen", w: 0, h: 0 },
  { label: "1080p", w: 1920, h: 1080 },
  { label: "720p", w: 1280, h: 720 },
];

const onDrag: MouseEventHandler = (e) => {
  if (e.button !== 0) return;
  const t = (e.target as HTMLElement).closest("button,select");
  if (t) return;
  e.preventDefault();
  getCurrentWindow().startDragging();
};

function Btn({ active, children, onClick, danger, style: s }: { active?: boolean; children: ReactNode; onClick: () => void; danger?: boolean; style?: CSSProperties }) {
  const [h, setH] = useState(false);
  const bg = active ? "rgba(255,255,255,0.14)" : h ? "rgba(255,255,255,0.07)" : "rgba(255,255,255,0.03)";
  const col = danger ? (h ? "#ff6b6b" : "#ff4757") : active ? "#f0f0f5" : h ? "#d4d4dc" : "#8b8b9e";
  return (
    <button onClick={onClick} onMouseEnter={() => setH(true)} onMouseLeave={() => setH(false)}
      style={{ padding: "5px 10px", borderRadius: 7, border: "none", background: bg, color: col, fontSize: 12, fontWeight: active ? 600 : 500, cursor: "pointer", fontFamily: "inherit", display: "flex", alignItems: "center", gap: 5, transition: "all 0.1s", whiteSpace: "nowrap", ...s }}
    >{children}</button>
  );
}

const Sep = () => <div style={{ width: 1, height: 22, background: "rgba(255,255,255,0.08)", flexShrink: 0 }} />;

export function RecorderPill() {
  const [mode, setMode] = useState<Mode>("screen");
  const [selScreen, setSelScreen] = useState(1);
  const [preset, setPreset] = useState(0);
  const [fps, setFps] = useState(30);
  const [recording, setRecording] = useState(false);
  const [seconds, setSeconds] = useState(0);
  const [showOpts, setShowOpts] = useState(false);

  useEffect(() => {
    invoke<ScreenDevice[]>("list_screens").then((s) => { if (s.length > 0) setSelScreen(s[0].index); }).catch(() => {});
  }, []);

  useEffect(() => {
    if (!recording) return;
    const t = setInterval(() => setSeconds((s) => s + 1), 1000);
    return () => clearInterval(t);
  }, [recording]);

  const doRecord = async () => {
    const p = PRESETS[preset];
    await emit("recorder-started", {});
    try { await getCurrentWindow().setSize(new LogicalSize(230, 48)); } catch {}
    setRecording(true); setSeconds(0);
    try {
      await invoke("start_recording", { config: {
        output_path: "/tmp/narrator_recording.mp4",
        screen_index: selScreen, width: p.w, height: p.h, fps,
        offset_x: 0, offset_y: 0, capture_audio: false,
      } as RecordingConfig });
    } catch (e) { console.error(e); }
  };

  const doStop = async () => {
    try {
      await invoke("stop_recording");
      setRecording(false);
      await new Promise((r) => setTimeout(r, 1200));
      await emit("recorder-stopped", { path: "/tmp/narrator_recording.mp4" });
      await invoke("close_recorder_window");
    } catch (e) { console.error(e); }
  };

  const doClose = () => invoke("close_recorder_window");
  const fmt = (s: number) => `${Math.floor(s / 60)}:${(s % 60).toString().padStart(2, "0")}`;

  const bar: CSSProperties = {
    height: "100%", display: "flex", alignItems: "center",
    padding: "0 10px", gap: 5, cursor: "grab",
    background: "#1c1c24",
    borderRadius: 14,
  };

  // ── RECORDING ──
  if (recording) {
    return (
      <div onMouseDown={onDrag} style={{ ...bar, justifyContent: "center", gap: 12, padding: "0 16px" }}>
        <div style={{ width: 10, height: 10, borderRadius: "50%", background: "#ff3b30", boxShadow: "0 0 10px rgba(255,59,48,0.5)", animation: "rp 1s infinite" }} />
        <span style={{ fontFamily: "SF Mono, Menlo, monospace", fontSize: 17, fontWeight: 700, color: "#ff3b30", letterSpacing: 0.5, minWidth: 44, textAlign: "center" }}>{fmt(seconds)}</span>
        <Sep />
        <Btn onClick={doStop} danger>
          <svg width="10" height="10" viewBox="0 0 16 16" fill="currentColor"><rect x="2" y="2" width="12" height="12" rx="2"/></svg>
          Stop
        </Btn>
        <style>{`@keyframes rp{0%,100%{opacity:1}50%{opacity:.3}}`}</style>
      </div>
    );
  }

  // ── SETUP ──
  return (
    <div onMouseDown={onDrag} style={bar}>
      {/* Close */}
      <Btn onClick={doClose} style={{ padding: "5px 6px" }}>
        <svg width="10" height="10" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><line x1="3" y1="3" x2="13" y2="13"/><line x1="13" y1="3" x2="3" y2="13"/></svg>
      </Btn>

      <Sep />

      {/* Modes */}
      <Btn active={mode === "screen"} onClick={() => setMode("screen")}>
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><rect x="2" y="3" width="20" height="14" rx="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/></svg>
        Entire Screen
      </Btn>
      <Btn active={mode === "window"} onClick={() => setMode("window")}>
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><rect x="3" y="3" width="18" height="18" rx="2"/><line x1="3" y1="9" x2="21" y2="9"/></svg>
        Window
      </Btn>
      <Btn active={mode === "region"} onClick={() => setMode("region")}>
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeDasharray="4 3"><rect x="4" y="4" width="16" height="16" rx="1"/></svg>
        Region
      </Btn>

      <Sep />

      {/* Options */}
      <div style={{ position: "relative" }}>
        <Btn active={showOpts} onClick={() => setShowOpts(!showOpts)}>
          Options
          <svg width="7" height="7" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="2"><path d="M2 4l4 4 4-4"/></svg>
        </Btn>
        {showOpts && (
          <div onClick={(e) => e.stopPropagation()} style={{
            position: "absolute", bottom: "calc(100% + 8px)", right: 0,
            background: "#22222c", borderRadius: 10, border: "1px solid rgba(255,255,255,0.1)",
            boxShadow: "0 12px 36px rgba(0,0,0,0.5)", padding: "12px 14px", width: 170,
            display: "flex", flexDirection: "column", gap: 10,
          }}>
            <div>
              <div style={{ fontSize: 10, color: "#6b6b80", fontWeight: 600, marginBottom: 5, textTransform: "uppercase", letterSpacing: 0.8 }}>Resolution</div>
              <div style={{ display: "flex", gap: 3 }}>
                {PRESETS.map((p, i) => (
                  <button key={i} onClick={() => setPreset(i)} style={{
                    flex: 1, padding: "4px 2px", borderRadius: 5, border: "none",
                    background: preset === i ? "rgba(255,255,255,0.12)" : "rgba(255,255,255,0.03)",
                    color: preset === i ? "#e0e0ea" : "#5a5a6e", fontSize: 10,
                    fontWeight: preset === i ? 600 : 400, cursor: "pointer", fontFamily: "inherit",
                  }}>{p.label}</button>
                ))}
              </div>
            </div>
            <div>
              <div style={{ fontSize: 10, color: "#6b6b80", fontWeight: 600, marginBottom: 5, textTransform: "uppercase", letterSpacing: 0.8 }}>FPS</div>
              <div style={{ display: "flex", gap: 3 }}>
                {[15, 30, 60].map((f) => (
                  <button key={f} onClick={() => setFps(f)} style={{
                    flex: 1, padding: "4px 2px", borderRadius: 5, border: "none",
                    background: fps === f ? "rgba(255,255,255,0.12)" : "rgba(255,255,255,0.03)",
                    color: fps === f ? "#e0e0ea" : "#5a5a6e", fontSize: 10,
                    fontWeight: fps === f ? 600 : 400, cursor: "pointer", fontFamily: "inherit",
                  }}>{f}</button>
                ))}
              </div>
            </div>
          </div>
        )}
      </div>

      {/* Record */}
      <Btn onClick={doRecord} style={{ background: "rgba(255,255,255,0.1)", color: "#e0e0ea", fontWeight: 600, padding: "6px 16px" }}>
        Record
      </Btn>
    </div>
  );
}
