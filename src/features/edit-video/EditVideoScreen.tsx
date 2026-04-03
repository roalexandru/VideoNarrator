import { useRef, useEffect, useState, useCallback } from "react";
import { Channel } from "@tauri-apps/api/core";
import { useProjectStore } from "../../stores/projectStore";
import { useEditStore } from "../../stores/editStore";
import { applyVideoEdits, extractEditThumbnails } from "../../lib/tauri/commands";
import { Button } from "../../components/ui/Button";
import { ProgressBar } from "../../components/ui/ProgressBar";
import { secondsToTimestamp } from "../../lib/formatters";
import { convertFileSrc } from "@tauri-apps/api/core";
import type { ProgressEvent } from "../../types/processing";

const C = { text: "#e0e0ea", dim: "#8b8ba0", muted: "#5a5a6e", border: "rgba(255,255,255,0.07)", accent: "#818cf8" };

export function EditVideoScreen() {
  const videoFile = useProjectStore((s) => s.videoFile);
  const projectId = useProjectStore((s) => s.projectId);
  const { clips, selectedClipIndex, editedVideoPath, initFromVideo, splitAt, deleteClip, setClipSpeed, setClipFps, selectClip, setEditedVideoPath } = useEditStore();

  const videoRef = useRef<HTMLVideoElement>(null);
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [isPlaying, setIsPlaying] = useState(false);
  const [thumbs, setThumbs] = useState<string[]>([]);
  const [applying, setApplying] = useState(false);
  const [applyPct, setApplyPct] = useState(0);
  const [videoHover, setVideoHover] = useState(false);

  const src = videoFile?.path ? convertFileSrc(videoFile.path) : undefined;
  const selClip = selectedClipIndex !== null ? clips[selectedClipIndex] : null;

  // Init timeline from video
  useEffect(() => {
    if (videoFile?.duration && clips.length === 0) initFromVideo(videoFile.duration);
  }, [videoFile?.duration, clips.length, initFromVideo]);

  // Load video
  useEffect(() => { const v = videoRef.current; if (v && src) { v.src = src; v.load(); } }, [src]);

  useEffect(() => {
    const v = videoRef.current; if (!v) return;
    const h = { time: () => setCurrentTime(v.currentTime), meta: () => setDuration(v.duration || 0), play: () => setIsPlaying(true), pause: () => setIsPlaying(false) };
    v.addEventListener("timeupdate", h.time); v.addEventListener("loadedmetadata", h.meta); v.addEventListener("play", h.play); v.addEventListener("pause", h.pause);
    return () => { v.removeEventListener("timeupdate", h.time); v.removeEventListener("loadedmetadata", h.meta); v.removeEventListener("play", h.play); v.removeEventListener("pause", h.pause); };
  }, []);

  // Extract thumbnails for editing timeline
  useEffect(() => {
    if (!videoFile?.path) return;
    // Use a temp dir for edit thumbnails — these are separate from processing frames
    const dir = `/tmp/narrator_edit_thumbs_${projectId || "tmp"}`;
    extractEditThumbnails(videoFile.path, dir, 50)
      .then((paths) => setThumbs(paths.map((p) => convertFileSrc(p))))
      .catch((e) => console.error("Thumbnail extraction:", e));
  }, [videoFile?.path, projectId]);

  // Keyboard
  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      const t = (e.target as HTMLElement).tagName;
      if (t === "INPUT" || t === "TEXTAREA") return;
      if (e.code === "Space") { e.preventDefault(); togglePlay(); }
      if (e.code === "KeyS") { e.preventDefault(); handleSplit(); }
      if ((e.code === "Delete" || e.code === "Backspace") && selectedClipIndex !== null) { e.preventDefault(); deleteClip(selectedClipIndex); }
    };
    window.addEventListener("keydown", h); return () => window.removeEventListener("keydown", h);
  }, [isPlaying, selectedClipIndex, currentTime]);

  const togglePlay = () => { const v = videoRef.current; if (!v) return; isPlaying ? v.pause() : v.play(); };
  const seekTo = (s: number) => { if (videoRef.current) videoRef.current.currentTime = s; };
  const handleSplit = useCallback(() => { if (currentTime > 0.5 && currentTime < duration - 0.5) splitAt(currentTime); }, [currentTime, duration, splitAt]);

  const handleApply = async () => {
    if (!videoFile?.path || clips.length === 0) return;
    setApplying(true); setApplyPct(0);
    try {
      const outPath = `/tmp/narrator_edited_${projectId}.mp4`;
      const ch = new Channel<ProgressEvent>();
      ch.onmessage = (e: ProgressEvent) => { if (e.kind === "progress") setApplyPct(e.percent); };
      const result = await applyVideoEdits(videoFile.path, outPath, {
        clips: clips.map((c) => ({ start_seconds: c.startSeconds, end_seconds: c.endSeconds, speed: c.speed, fps_override: c.fpsOverride })),
      }, ch);
      setEditedVideoPath(result);
    } catch (err: any) { console.error("Apply failed:", err); }
    finally { setApplying(false); }
  };

  const activeClipIdx = clips.findIndex((c) => currentTime >= c.startSeconds && currentTime < c.endSeconds);
  const editedDur = clips.reduce((s, c) => s + (c.endSeconds - c.startSeconds) / c.speed, 0);

  // Get thumbnail for a time position
  const getThumb = (seconds: number): string | undefined => {
    if (thumbs.length === 0 || duration <= 0) return undefined;
    return thumbs[Math.min(Math.floor((seconds / duration) * thumbs.length), thumbs.length - 1)];
  };

  return (
    <div style={{ height: "100%", display: "flex", flexDirection: "column" }}>
      {/* Header */}
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 8, flexShrink: 0 }}>
        <div>
          <h2 style={{ fontSize: 20, fontWeight: 700, color: C.text }}>Edit Video</h2>
          <p style={{ color: C.muted, fontSize: 12 }}>
            {clips.length} clip{clips.length !== 1 ? "s" : ""} &middot; {secondsToTimestamp(editedDur)} output
            {editedVideoPath && <span style={{ color: "#4ade80", marginLeft: 8 }}>Applied</span>}
          </p>
        </div>
        <div style={{ display: "flex", gap: 8 }}>
          <Button variant="secondary" size="sm" onClick={() => { useEditStore.getState().reset(); if (videoFile?.duration) initFromVideo(videoFile.duration); }}>Reset</Button>
          <Button size="sm" onClick={handleApply} disabled={applying || clips.length === 0}>{applying ? "Applying..." : "Apply Edits"}</Button>
        </div>
      </div>

      {applying && <ProgressBar value={applyPct} height={3} />}

      {/* VIDEO */}
      <div style={{ position: "relative", borderRadius: 8, overflow: "hidden", background: "#000", flexShrink: 0, aspectRatio: "16/9", maxHeight: "40vh" }}
        onMouseEnter={() => setVideoHover(true)} onMouseLeave={() => setVideoHover(false)}>
        <video ref={videoRef} playsInline onClick={togglePlay} style={{ width: "100%", height: "100%", objectFit: "contain", display: src ? "block" : "none" }} />
        {!src && <div style={{ width: "100%", height: "100%", display: "flex", alignItems: "center", justifyContent: "center", color: C.muted }}>Select a video in Project Setup first</div>}
        <div style={{ position: "absolute", bottom: 0, left: 0, right: 0, padding: "14px 12px 6px", background: "linear-gradient(transparent, rgba(0,0,0,0.7))", opacity: videoHover ? 1 : 0, transition: "opacity 0.15s", display: "flex", alignItems: "center", gap: 8 }}>
          <button onClick={togglePlay} style={{ background: "none", border: "none", cursor: "pointer", color: "#fff", display: "flex" }}>
            {isPlaying ? <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="4" width="4" height="16"/><rect x="14" y="4" width="4" height="16"/></svg>
              : <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor"><polygon points="5 3 19 12 5 21 5 3"/></svg>}
          </button>
          <span style={{ color: "#fff", fontSize: 11, fontFamily: "monospace" }}>{secondsToTimestamp(currentTime)} / {secondsToTimestamp(duration)}</span>
        </div>
      </div>

      {/* TIMELINE + TOOLS */}
      <div style={{ flex: 1, display: "flex", flexDirection: "column", marginTop: 10, minHeight: 0 }}>
        {/* Scrubber */}
        <div onClick={(e) => { if (!duration) return; const r = e.currentTarget.getBoundingClientRect(); seekTo(((e.clientX - r.left) / r.width) * duration); }}
          style={{ height: 4, background: "rgba(255,255,255,0.06)", borderRadius: 2, cursor: "pointer", position: "relative", marginBottom: 6, flexShrink: 0 }}>
          <div style={{ height: "100%", width: duration > 0 ? `${(currentTime / duration) * 100}%` : "0%", background: C.accent, borderRadius: 2 }} />
          {duration > 0 && <div style={{ position: "absolute", top: -5, left: `${(currentTime / duration) * 100}%`, transform: "translateX(-50%)", width: 14, height: 14, borderRadius: "50%", background: "#ef4444", border: "2px solid #0f0f15" }} />}
        </div>

        {/* Thumbnail timeline with clip regions */}
        <div style={{ position: "relative", height: 56, borderRadius: 6, overflow: "hidden", flexShrink: 0, marginBottom: 8 }}>
          {/* Background thumbnails */}
          <div style={{ display: "flex", height: "100%", gap: 0 }}>
            {thumbs.length > 0 ? thumbs.map((t, i) => (
              <div key={i} style={{ flex: 1, backgroundImage: `url(${t})`, backgroundSize: "cover", backgroundPosition: "center", minWidth: 1, filter: "brightness(0.3)" }} />
            )) : (
              <div style={{ flex: 1, background: "rgba(255,255,255,0.03)" }} />
            )}
          </div>

          {/* Clip overlay regions (bright) */}
          {clips.map((clip, i) => {
            const left = duration > 0 ? (clip.startSeconds / duration) * 100 : 0;
            const width = duration > 0 ? ((clip.endSeconds - clip.startSeconds) / duration) * 100 : 100;
            const isSelected = i === selectedClipIndex;
            const isActive = i === activeClipIdx;
            return (
              <div key={clip.id} onClick={() => { selectClip(i); seekTo(clip.startSeconds); }}
                style={{
                  position: "absolute", top: 0, bottom: 0, left: `${left}%`, width: `${width}%`,
                  display: "flex", overflow: "hidden", cursor: "pointer",
                  border: isSelected ? "2px solid #6366f1" : isActive ? "1px solid rgba(99,102,241,0.4)" : "1px solid rgba(255,255,255,0.1)",
                  borderRadius: 3, boxSizing: "border-box",
                }}>
                {/* Show actual thumbnails for this clip */}
                {thumbs.length > 0 && (() => {
                  const startIdx = Math.floor((clip.startSeconds / duration) * thumbs.length);
                  const endIdx = Math.ceil((clip.endSeconds / duration) * thumbs.length);
                  const clipThumbs = thumbs.slice(startIdx, endIdx);
                  return clipThumbs.map((t, j) => (
                    <div key={j} style={{ flex: 1, backgroundImage: `url(${t})`, backgroundSize: "cover", backgroundPosition: "center", minWidth: 1 }} />
                  ));
                })()}
                {/* Speed/FPS badge */}
                {(clip.speed !== 1.0 || clip.fpsOverride) && (
                  <div style={{ position: "absolute", bottom: 2, left: 4, display: "flex", gap: 3 }}>
                    {clip.speed !== 1.0 && <span style={{ fontSize: 9, padding: "1px 4px", borderRadius: 3, background: "rgba(168,85,247,0.8)", color: "#fff", fontWeight: 700 }}>{clip.speed}x</span>}
                    {clip.fpsOverride && <span style={{ fontSize: 9, padding: "1px 4px", borderRadius: 3, background: "rgba(245,158,11,0.8)", color: "#fff", fontWeight: 700 }}>{clip.fpsOverride}fps</span>}
                  </div>
                )}
                {/* Timestamp */}
                <div style={{ position: "absolute", top: 2, left: 4, fontSize: 9, color: "rgba(255,255,255,0.8)", fontFamily: "monospace", textShadow: "0 1px 2px rgba(0,0,0,0.5)" }}>
                  {secondsToTimestamp(clip.startSeconds)}
                </div>
              </div>
            );
          })}

          {/* Playhead */}
          {duration > 0 && (
            <div style={{ position: "absolute", top: 0, bottom: 0, left: `${(currentTime / duration) * 100}%`, width: 2, background: "#ef4444", zIndex: 10, pointerEvents: "none" }}>
              <div style={{ width: 8, height: 8, background: "#ef4444", borderRadius: "50%", position: "absolute", top: -4, left: -3 }} />
            </div>
          )}
        </div>

        {/* Tools + Clip list */}
        <div style={{ flex: 1, display: "grid", gridTemplateColumns: "160px 1fr", gap: 10, minHeight: 0, overflowY: "auto" }}>
          {/* Tools */}
          <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
            <div style={{ fontSize: 10, fontWeight: 700, color: C.muted, textTransform: "uppercase", letterSpacing: 1 }}>Tools</div>

            <button onClick={handleSplit} style={{ display: "flex", alignItems: "center", gap: 6, padding: "7px 8px", borderRadius: 6, border: `1px solid ${C.border}`, background: "rgba(255,255,255,0.02)", color: C.dim, fontSize: 12, cursor: "pointer", fontFamily: "inherit", textAlign: "left" }}>
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><circle cx="6" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><line x1="20" y1="4" x2="8.12" y2="15.88"/><line x1="14.47" y1="14.48" x2="20" y2="20"/></svg>
              Split (S)
            </button>

            <button onClick={() => { if (selectedClipIndex !== null) deleteClip(selectedClipIndex); }} disabled={selectedClipIndex === null || clips.length <= 1}
              style={{ display: "flex", alignItems: "center", gap: 6, padding: "7px 8px", borderRadius: 6, border: `1px solid ${C.border}`, background: "rgba(255,255,255,0.02)", color: selectedClipIndex !== null && clips.length > 1 ? "#f87171" : "#2a2a3a", fontSize: 12, cursor: "pointer", fontFamily: "inherit", textAlign: "left" }}>
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2"/></svg>
              Delete (Del)
            </button>

            {selClip && (
              <>
                <div style={{ padding: "8px", borderRadius: 6, border: `1px solid ${C.border}`, background: "rgba(255,255,255,0.02)" }}>
                  <div style={{ fontSize: 10, fontWeight: 600, color: C.dim, marginBottom: 4 }}>Speed ({selClip.speed.toFixed(1)}x)</div>
                  <input type="range" min="0.25" max="4" step="0.25" value={selClip.speed}
                    onChange={(e) => setClipSpeed(selectedClipIndex!, parseFloat(e.target.value))}
                    style={{ width: "100%", accentColor: "#6366f1" }} />
                  <div style={{ display: "flex", justifyContent: "space-between", fontSize: 9, color: C.muted }}><span>0.25x</span><span>1x</span><span>4x</span></div>
                </div>

                <div style={{ padding: "8px", borderRadius: 6, border: `1px solid ${C.border}`, background: "rgba(255,255,255,0.02)" }}>
                  <div style={{ fontSize: 10, fontWeight: 600, color: C.dim, marginBottom: 4 }}>FPS ({selClip.fpsOverride ? `${selClip.fpsOverride}` : "Original"})</div>
                  <input type="range" min="0" max="30" step="5" value={selClip.fpsOverride || 0}
                    onChange={(e) => { const v = parseInt(e.target.value); setClipFps(selectedClipIndex!, v === 0 ? null : v); }}
                    style={{ width: "100%", accentColor: "#6366f1" }} />
                  <div style={{ display: "flex", justifyContent: "space-between", fontSize: 9, color: C.muted }}><span>Orig</span><span>15</span><span>30</span></div>
                  <p style={{ fontSize: 9, color: C.muted, marginTop: 3 }}>Skip loading screens</p>
                </div>
              </>
            )}
          </div>

          {/* Clip list */}
          <div style={{ overflowY: "auto" }}>
            <div style={{ fontSize: 10, fontWeight: 700, color: C.muted, textTransform: "uppercase", letterSpacing: 1, marginBottom: 6 }}>Clips ({clips.length})</div>
            <div style={{ display: "flex", flexDirection: "column", gap: 3 }}>
              {clips.map((clip, i) => {
                const isSel = i === selectedClipIndex;
                const dur = (clip.endSeconds - clip.startSeconds) / clip.speed;
                const thumb = getThumb(clip.startSeconds);
                return (
                  <div key={clip.id} onClick={() => { selectClip(i); seekTo(clip.startSeconds); }}
                    style={{
                      display: "flex", alignItems: "center", gap: 10, padding: "6px 8px", borderRadius: 6, cursor: "pointer",
                      background: isSel ? "rgba(99,102,241,0.08)" : "transparent",
                      borderLeft: isSel ? "3px solid #6366f1" : "3px solid transparent",
                    }}>
                    {/* Mini thumbnail */}
                    <div style={{ width: 48, height: 28, borderRadius: 4, overflow: "hidden", flexShrink: 0, background: "#1a1a24" }}>
                      {thumb && <img src={thumb} style={{ width: "100%", height: "100%", objectFit: "cover" }} />}
                    </div>
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div style={{ fontFamily: "monospace", fontSize: 11, color: isSel ? C.accent : C.dim }}>
                        {secondsToTimestamp(clip.startSeconds)} - {secondsToTimestamp(clip.endSeconds)}
                      </div>
                      <div style={{ fontSize: 10, color: C.muted }}>{dur.toFixed(1)}s output</div>
                    </div>
                    <div style={{ display: "flex", gap: 4, flexShrink: 0 }}>
                      {clip.speed !== 1.0 && <span style={{ fontSize: 9, padding: "1px 5px", borderRadius: 3, background: "rgba(168,85,247,0.12)", color: "#a855f7", fontWeight: 700 }}>{clip.speed}x</span>}
                      {clip.fpsOverride && <span style={{ fontSize: 9, padding: "1px 5px", borderRadius: 3, background: "rgba(245,158,11,0.12)", color: "#f59e0b", fontWeight: 700 }}>{clip.fpsOverride}fps</span>}
                    </div>
                  </div>
                );
              })}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
