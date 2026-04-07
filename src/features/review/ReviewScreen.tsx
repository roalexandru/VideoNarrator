import { useRef, useEffect, useCallback, useState } from "react";
import { useScriptStore } from "../../stores/scriptStore";
import { useProjectStore } from "../../stores/projectStore";
import { useConfigStore } from "../../stores/configStore";
import { secondsToTimestamp } from "../../lib/formatters";
import { listProjectFrames } from "../../lib/tauri/commands";
import { convertFileSrc } from "@tauri-apps/api/core";
import { ConfirmDialog } from "../../components/ui/ConfirmDialog";

const C = { text: "#e0e0ea", dim: "#8b8ba0", muted: "#5a5a6e", border: "rgba(255,255,255,0.07)", accent: "#818cf8" };

export function ReviewScreen() {
  const videoFile = useProjectStore((s) => s.videoFile);
  const projectId = useProjectStore((s) => s.projectId);
  const languages = useConfigStore((s) => s.languages);
  const { scripts, activeLanguage, setActiveLanguage, setActiveSegment, updateSegmentText, deleteSegment } = useScriptStore();
  const videoRef = useRef<HTMLVideoElement>(null);
  const timelineRef = useRef<HTMLDivElement>(null);
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [isPlaying, setIsPlaying] = useState(false);
  const [framePaths, setFramePaths] = useState<string[]>([]);
  const [deleteTarget, setDeleteTarget] = useState<number | null>(null);

  const script = scripts[activeLanguage];
  const segments = script?.segments || [];
  const src = videoFile?.path ? convertFileSrc(videoFile.path) : undefined;
  const currentSegmentIdx = segments.findIndex((s) => currentTime >= s.start_seconds && currentTime < s.end_seconds);
  const currentSegment = currentSegmentIdx >= 0 ? segments[currentSegmentIdx] : null;

  // Load frame thumbnails
  useEffect(() => {
    if (!projectId) return;
    listProjectFrames(projectId).then((frames) => {
      setFramePaths(frames.map((f) => convertFileSrc(f.path)));
    }).catch(() => {});
  }, [projectId]);

  // Get thumbnail for a given timestamp
  const getThumbnail = useCallback((seconds: number): string | undefined => {
    if (framePaths.length === 0 || duration <= 0) return undefined;
    // Frames are evenly spaced across the video duration
    const frameInterval = duration / framePaths.length;
    const idx = Math.min(Math.floor(seconds / frameInterval), framePaths.length - 1);
    return framePaths[idx];
  }, [framePaths, duration]);

  const handleSegmentClick = useCallback((i: number) => {
    setActiveSegment(i);
    if (segments[i] && videoRef.current) videoRef.current.currentTime = segments[i].start_seconds;
  }, [segments, setActiveSegment]);

  // Load video
  useEffect(() => {
    const v = videoRef.current;
    if (v && src) { v.src = src; v.load(); }
  }, [src]);

  useEffect(() => {
    const v = videoRef.current; if (!v) return;
    const onTime = () => setCurrentTime(v.currentTime);
    const onMeta = () => setDuration(v.duration || 0);
    const onPlay = () => setIsPlaying(true);
    const onPause = () => setIsPlaying(false);
    v.addEventListener("timeupdate", onTime);
    v.addEventListener("loadedmetadata", onMeta);
    v.addEventListener("play", onPlay);
    v.addEventListener("pause", onPause);
    return () => { v.removeEventListener("timeupdate", onTime); v.removeEventListener("loadedmetadata", onMeta); v.removeEventListener("play", onPlay); v.removeEventListener("pause", onPause); };
  }, []);

  // Space bar to toggle play/pause (skip if user is typing in a textarea/input)
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.code === "Space") {
        const tag = (e.target as HTMLElement).tagName;
        if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
        e.preventDefault();
        togglePlay();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [isPlaying]);

  // Auto-scroll timeline
  useEffect(() => {
    if (currentSegmentIdx >= 0 && timelineRef.current) {
      const el = timelineRef.current.children[currentSegmentIdx] as HTMLElement;
      if (el) el.scrollIntoView({ behavior: "smooth", block: "nearest", inline: "center" });
    }
  }, [currentSegmentIdx]);

  const seekTo = (s: number) => { if (videoRef.current) videoRef.current.currentTime = s; };
  const togglePlay = () => { if (!videoRef.current) return; isPlaying ? videoRef.current.pause() : videoRef.current.play(); };

  const timelineClick = (e: React.MouseEvent<HTMLDivElement>) => {
    if (!duration) return;
    const rect = e.currentTarget.getBoundingClientRect();
    seekTo(((e.clientX - rect.left) / rect.width) * duration);
  };

  return (
    <div style={{ height: "100%", display: "flex", flexDirection: "column" }}>
      {/* Header */}
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 10, flexShrink: 0 }}>
        <h2 style={{ fontSize: 20, fontWeight: 700, color: C.text }}>Review & Edit</h2>
        {languages.length > 1 && (
          <div style={{ display: "flex", gap: 2, background: "rgba(255,255,255,0.04)", borderRadius: 8, padding: 3 }}>
            {languages.map((l) => (
              <button key={l} onClick={() => setActiveLanguage(l)} style={{
                padding: "4px 12px", borderRadius: 6, border: "none", fontFamily: "inherit",
                background: activeLanguage === l ? "rgba(99,102,241,0.15)" : "transparent",
                color: activeLanguage === l ? C.accent : C.muted, fontSize: 12, fontWeight: 600, cursor: "pointer",
              }}>{l.toUpperCase()}</button>
            ))}
          </div>
        )}
      </div>

      {/* VIDEO PLAYER — controls hidden until hover */}
      <div
        style={{ position: "relative", borderRadius: 10, overflow: "hidden", background: "#000", flexShrink: 0, aspectRatio: "16/9", maxHeight: "42vh" }}
      >
        <video ref={videoRef} playsInline onClick={togglePlay}
          style={{ width: "100%", height: "100%", objectFit: "contain", display: src ? "block" : "none" }} />
        {!src && <div style={{ width: "100%", height: "100%", display: "flex", alignItems: "center", justifyContent: "center", color: C.muted }}>No video</div>}

        {/* Caption overlay */}
        {currentSegment && (
          <div style={{
            position: "absolute", bottom: 56, left: "50%", transform: "translateX(-50%)",
            maxWidth: "80%", padding: "10px 20px",
            background: "rgba(0,0,0,0.75)", borderRadius: 8, transition: "bottom 0.2s",
          }}>
            <p style={{ color: "#fff", fontSize: 15, textAlign: "center", lineHeight: 1.5, margin: 0 }}>{currentSegment.text}</p>
          </div>
        )}

        {/* Custom controls bar — always visible */}
        <div style={{
          position: "absolute", bottom: 0, left: 0, right: 0,
          background: "linear-gradient(transparent, rgba(0,0,0,0.7))",
          padding: "20px 16px 10px",
          opacity: 1,
          display: "flex", alignItems: "center", gap: 12,
        }}>
          <button onClick={togglePlay} style={{ background: "none", border: "none", cursor: "pointer", padding: 4, color: "#fff", display: "flex" }}>
            {isPlaying
              ? <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="4" width="4" height="16"/><rect x="14" y="4" width="4" height="16"/></svg>
              : <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor"><polygon points="5 3 19 12 5 21 5 3"/></svg>}
          </button>
          <span style={{ color: "#fff", fontSize: 12, fontFamily: "monospace", minWidth: 90 }}>
            {secondsToTimestamp(currentTime)} / {secondsToTimestamp(duration)}
          </span>
          {/* Mini scrubber */}
          <div onClick={timelineClick} style={{ flex: 1, height: 4, background: "rgba(255,255,255,0.2)", borderRadius: 2, cursor: "pointer", position: "relative" }}>
            <div style={{ height: "100%", width: duration > 0 ? `${(currentTime / duration) * 100}%` : "0%", background: C.accent, borderRadius: 2 }} />
          </div>
        </div>

        {/* Time badge — hidden since controls are always visible */}
      </div>

      {/* THUMBNAIL TIMELINE */}
      <div style={{ flexShrink: 0, padding: "6px 0" }}>
        {/* Scrubber track */}
        <div onClick={timelineClick} style={{ height: 4, background: "rgba(255,255,255,0.06)", borderRadius: 2, cursor: "pointer", position: "relative", marginBottom: 6 }}>
          <div style={{ height: "100%", width: duration > 0 ? `${(currentTime / duration) * 100}%` : "0%", background: C.accent, borderRadius: 2, transition: "width 0.1s" }} />
          {duration > 0 && (
            <div style={{
              position: "absolute", top: -5, left: `${(currentTime / duration) * 100}%`, transform: "translateX(-50%)",
              width: 14, height: 14, borderRadius: "50%", background: C.accent,
              border: "2px solid #0f0f15", boxShadow: `0 0 6px ${C.accent}`,
            }} />
          )}
        </div>

        {/* Segment blocks with thumbnails */}
        <div ref={timelineRef} style={{ display: "flex", gap: 2, overflowX: "auto", paddingBottom: 4 }}>
          {segments.map((seg, i) => {
            const isCurrent = i === currentSegmentIdx;
            const widthPct = duration > 0 ? ((seg.end_seconds - seg.start_seconds) / duration) * 100 : 10;
            const thumb = getThumbnail(seg.start_seconds);
            return (
              <div key={i} onClick={() => handleSegmentClick(i)} style={{
                flex: `0 0 ${Math.max(widthPct, 4)}%`, height: 48, borderRadius: 4, cursor: "pointer",
                border: isCurrent ? "2px solid rgba(99,102,241,0.7)" : "1px solid rgba(255,255,255,0.06)",
                overflow: "hidden", position: "relative",
                backgroundImage: thumb ? `url(${thumb})` : "none",
                backgroundSize: "cover", backgroundPosition: "center",
                backgroundColor: thumb ? "transparent" : "rgba(255,255,255,0.04)",
                transition: "border-color 0.1s",
              }}>
                {/* Darken overlay + timestamp */}
                <div style={{
                  position: "absolute", inset: 0,
                  background: isCurrent ? "rgba(99,102,241,0.3)" : "rgba(0,0,0,0.4)",
                  display: "flex", alignItems: "center", justifyContent: "center",
                }}>
                  <span style={{
                    fontSize: 10, fontFamily: "monospace", fontWeight: 600,
                    color: isCurrent ? "#fff" : "rgba(255,255,255,0.8)",
                    textShadow: "0 1px 3px rgba(0,0,0,0.5)",
                  }}>
                    {secondsToTimestamp(seg.start_seconds)}
                  </span>
                </div>
              </div>
            );
          })}
        </div>
      </div>

      {/* SEGMENT EDITOR */}
      <div style={{ flex: 1, minHeight: 0, overflowY: "auto", borderTop: `1px solid ${C.border}`, paddingTop: 10 }}>
        {segments.length === 0 ? (
          <div style={{ textAlign: "center", padding: 48, color: C.muted }}>
            <svg width="36" height="36" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" style={{ margin: "0 auto 12px", display: "block", opacity: 0.5 }}>
              <path d="M21 15a2 2 0 01-2 2H7l-4 4V5a2 2 0 012-2h14a2 2 0 012 2z"/>
            </svg>
            <div style={{ fontSize: 15, fontWeight: 600, color: C.dim, marginBottom: 6 }}>No narration generated yet</div>
            <div style={{ fontSize: 13 }}>Go to Processing to generate narration for your video.</div>
          </div>
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
            {segments.map((seg, i) => {
              const isCurrent = i === currentSegmentIdx;
              return (
                <div key={i} onClick={() => handleSegmentClick(i)} style={{
                  display: "grid", gridTemplateColumns: "80px 1fr auto", gap: 12, alignItems: "start",
                  padding: "10px 14px", borderRadius: 6, cursor: "pointer",
                  background: isCurrent ? "rgba(99,102,241,0.06)" : "transparent",
                  borderLeft: isCurrent ? "3px solid #6366f1" : "3px solid transparent",
                }}>
                  <div style={{ fontFamily: "monospace", fontSize: 12, color: isCurrent ? C.accent : C.muted, fontWeight: 600, paddingTop: 2 }}>
                    {secondsToTimestamp(seg.start_seconds)}
                    <div style={{ fontSize: 10, opacity: 0.6 }}>{secondsToTimestamp(seg.end_seconds)}</div>
                  </div>
                  <div>
                    <textarea value={seg.text}
                      onChange={(e) => updateSegmentText(activeLanguage, i, e.target.value)}
                      onClick={(e) => e.stopPropagation()}
                      aria-label={`Narration text for segment ${i + 1}`}
                      rows={2}
                      style={{ width: "100%", fontSize: 13, color: isCurrent ? C.text : C.dim, background: "rgba(255,255,255,0.04)", border: `1px solid ${isCurrent ? "rgba(99,102,241,0.3)" : C.border}`, borderRadius: 6, padding: "8px 10px", outline: "none", resize: "none" as const, lineHeight: 1.5, fontFamily: "inherit" }}
                    />
                    {seg.visual_description && (
                      <p style={{ fontSize: 11, color: C.muted, marginTop: 4, fontStyle: "italic" }}>{seg.visual_description}</p>
                    )}
                  </div>
                  <div style={{ display: "flex", gap: 4, paddingTop: 2 }}>
                    <span style={{ fontSize: 10, background: "rgba(255,255,255,0.05)", padding: "2px 6px", borderRadius: 4, color: C.muted }}>{seg.pace}</span>
                    <button onClick={(e) => { e.stopPropagation(); setDeleteTarget(i); }} style={{ fontSize: 11, color: "#f87171", background: "none", border: "none", cursor: "pointer", fontFamily: "inherit" }}>Del</button>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>

      {deleteTarget !== null && (
        <ConfirmDialog
          title="Delete Segment"
          message={`Are you sure you want to delete segment ${deleteTarget + 1}? This cannot be undone.`}
          confirmLabel="Delete"
          onConfirm={() => { deleteSegment(activeLanguage, deleteTarget); setDeleteTarget(null); }}
          onCancel={() => setDeleteTarget(null)}
        />
      )}
    </div>
  );
}
