import { useRef, useEffect, useState, useCallback } from "react";
import { useProjectStore } from "../../stores/projectStore";
import { useEditStore } from "../../stores/editStore";
import { extractEditThumbnails } from "../../lib/tauri/commands";
import { Button } from "../../components/ui/Button";
import { secondsToTimestamp } from "../../lib/formatters";
import { convertFileSrc } from "@tauri-apps/api/core";

const C = { text: "#e0e0ea", dim: "#8b8ba0", muted: "#5a5a6e", border: "rgba(255,255,255,0.07)", accent: "#818cf8" };

export function EditVideoScreen() {
  const videoFile = useProjectStore((s) => s.videoFile);
  const projectId = useProjectStore((s) => s.projectId);
  const store = useEditStore();
  const { clips, selectedClipIndex, initFromVideo, splitAt, deleteClip, setClipSpeed, setClipSpeedLive, commitSpeedChange, setClipSkipFrames, moveClip, selectClip, undo, redo, canUndo, canRedo } = store;

  const videoRef = useRef<HTMLVideoElement>(null);
  const timelineRef = useRef<HTMLDivElement>(null);
  const dragHandlersRef = useRef<{ onMove: (e: MouseEvent) => void; onUp: () => void } | null>(null);
  const rafRef = useRef<number>(0);
  const [videoDuration, setVideoDuration] = useState(0); // actual source video duration
  const [isPlaying, setIsPlaying] = useState(false);
  const [outputTime, setOutputTime] = useState(0); // position on the OUTPUT timeline
  const [thumbs, setThumbs] = useState<string[]>([]);
  const [zoom, setZoom] = useState(0);
  const [dragging, setDragging] = useState(false);
  const [timelineHeight, setTimelineHeight] = useState(() => {
    try {
      const saved = localStorage.getItem("narrator_timeline_height");
      return saved ? parseInt(saved) : 130;
    } catch {
      return 130;
    }
  });
  const [resizingTimeline, setResizingTimeline] = useState(false);
  const [dragClipIdx, setDragClipIdx] = useState<number | null>(null);

  // Persist timeline height
  useEffect(() => { try { localStorage.setItem("narrator_timeline_height", String(timelineHeight)); } catch { /* storage full or unavailable */ } }, [timelineHeight]);

  const src = videoFile?.path ? convertFileSrc(videoFile.path) : undefined;
  const selClip = selectedClipIndex !== null ? clips[selectedClipIndex] : null;
  const outputDuration = store.getOutputDuration();

  // Init clips only when no edits exist — preserves edits on navigation
  useEffect(() => {
    if (videoDuration > 0 && clips.length === 0) {
      initFromVideo(videoDuration);
    }
  }, [videoDuration]);

  // Load video
  useEffect(() => { const v = videoRef.current; if (v && src) { v.src = src; v.load(); } }, [src]);

  useEffect(() => {
    const v = videoRef.current; if (!v) return;
    const onMeta = () => setVideoDuration(v.duration || 0);
    const onTime = () => {
      if (dragging) return;
      // Convert source time back to output time
      const sourceT = v.currentTime;
      let cumOut = 0;
      for (const clip of clips) {
        const clipSourceDur = clip.sourceEnd - clip.sourceStart;
        if (sourceT >= clip.sourceStart && sourceT <= clip.sourceEnd) {
          cumOut += (sourceT - clip.sourceStart) / clip.speed;
          setOutputTime(cumOut);
          return;
        }
        cumOut += clipSourceDur / clip.speed;
      }
      setOutputTime(cumOut);
    };
    const onPlay = () => setIsPlaying(true);
    const onPause = () => setIsPlaying(false);
    v.addEventListener("loadedmetadata", onMeta); v.addEventListener("timeupdate", onTime);
    v.addEventListener("play", onPlay); v.addEventListener("pause", onPause);
    return () => { v.removeEventListener("loadedmetadata", onMeta); v.removeEventListener("timeupdate", onTime); v.removeEventListener("play", onPlay); v.removeEventListener("pause", onPause); };
  }, [dragging, clips]);

  // Extract thumbnails
  useEffect(() => {
    if (!videoFile?.path) return;
    // Scale thumbnail count: short videos get more detail, long videos fewer to stay fast
    const dur = videoFile.duration || 120;
    const thumbCount = dur > 600 ? 30 : dur > 300 ? 40 : 60;
    extractEditThumbnails(videoFile.path, `/tmp/narrator_edit_thumbs_${projectId || crypto.randomUUID()}`, thumbCount)
      .then((paths) => setThumbs(paths.map((p) => convertFileSrc(p)))).catch(() => {});
  }, [videoFile?.path, projectId]);

  // Seek video to an output timeline position + set correct playback rate
  const seekToOutput = useCallback((t: number) => {
    const clamped = Math.max(0, Math.min(outputDuration, t));
    setOutputTime(clamped);
    const sourceT = store.outputTimeToSource(clamped);
    if (videoRef.current) {
      videoRef.current.currentTime = sourceT;
      // Find which clip we're in and set playback rate
      let cum = 0;
      for (const clip of clips) {
        const d = (clip.sourceEnd - clip.sourceStart) / clip.speed;
        if (clamped < cum + d) {
          videoRef.current.playbackRate = clip.speed;
          break;
        }
        cum += d;
      }
    }
  }, [outputDuration, store, clips]);

  // During playback: handle clip transitions (skip gaps, change speed at boundaries)
  useEffect(() => {
    if (!isPlaying || !videoRef.current) return;
    const v = videoRef.current;
    const tick = () => {
      const sourceT = v.currentTime;
      // Find which clip contains current source time
      let inClip = false;
      for (let i = 0; i < clips.length; i++) {
        const clip = clips[i];
        if (sourceT >= clip.sourceStart - 0.1 && sourceT <= clip.sourceEnd + 0.1) {
          // In this clip — set correct playback rate
          if (Math.abs(v.playbackRate - clip.speed) > 0.01) {
            v.playbackRate = clip.speed;
          }
          // If we've reached the end of this clip, jump to next clip's source start
          if (sourceT >= clip.sourceEnd - 0.05 && i < clips.length - 1) {
            v.currentTime = clips[i + 1].sourceStart;
            v.playbackRate = clips[i + 1].speed;
          }
          inClip = true;
          break;
        }
      }
      // If source time is between clips (in a deleted section), skip to the next clip
      if (!inClip) {
        for (const clip of clips) {
          if (sourceT < clip.sourceStart) {
            v.currentTime = clip.sourceStart;
            v.playbackRate = clip.speed;
            break;
          }
        }
      }
      // If past the last clip, pause
      if (clips.length > 0 && sourceT >= clips[clips.length - 1].sourceEnd) {
        v.pause();
      }
      if (isPlaying) {
        rafRef.current = requestAnimationFrame(tick);
      }
    };
    rafRef.current = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(rafRef.current);
  }, [isPlaying, clips]);

  // Keyboard
  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement).tagName;
      if (tag === "INPUT" || tag === "TEXTAREA") return;
      if (e.code === "Space") { e.preventDefault(); togglePlay(); }
      if (e.code === "KeyS") { e.preventDefault(); splitAt(outputTime); }
      if ((e.code === "Delete" || e.code === "Backspace") && selectedClipIndex !== null && clips.length > 1) { e.preventDefault(); deleteClip(selectedClipIndex); }
      if (e.code === "ArrowLeft") { e.preventDefault(); seekToOutput(outputTime - 1); }
      if (e.code === "ArrowRight") { e.preventDefault(); seekToOutput(outputTime + 1); }
      if ((e.metaKey || e.ctrlKey) && e.code === "KeyZ" && !e.shiftKey) { e.preventDefault(); undo(); }
      if ((e.metaKey || e.ctrlKey) && e.code === "KeyZ" && e.shiftKey) { e.preventDefault(); redo(); }
    };
    window.addEventListener("keydown", h); return () => window.removeEventListener("keydown", h);
  }, [isPlaying, selectedClipIndex, outputTime, clips, seekToOutput, splitAt, deleteClip, undo, redo]);

  const togglePlay = () => { const v = videoRef.current; if (!v) return; isPlaying ? v.pause() : v.play(); };

  // Timeline layout: clips are contiguous, no gaps
  const timelineWidth = timelineRef.current?.clientWidth || 800;
  const pxPerSec = zoom > 0 ? zoom : (outputDuration > 0 ? timelineWidth / outputDuration : 1);
  const totalPx = outputDuration * pxPerSec;

  // Click timeline → seek. Use pxPerSec directly, not DOM measurements.
  const handleTimelineClick = (e: React.MouseEvent) => {
    if (!timelineRef.current || outputDuration <= 0 || dragging) return;
    const rect = timelineRef.current.getBoundingClientRect();
    const x = e.clientX - rect.left + timelineRef.current.scrollLeft;
    const t = x / pxPerSec; // direct conversion — no DOM width needed
    seekToOutput(Math.max(0, Math.min(outputDuration, t)));
    let cum = 0;
    for (let i = 0; i < clips.length; i++) {
      const d = (clips[i].sourceEnd - clips[i].sourceStart) / clips[i].speed;
      if (t >= cum && t < cum + d) { selectClip(i); break; }
      cum += d;
    }
  };

  // Ctrl+Scroll zoom
  const handleWheel = (e: React.WheelEvent) => {
    if (e.ctrlKey || e.metaKey) {
      e.preventDefault();
      setZoom(Math.max(0.5, Math.min(50, (zoom || pxPerSec) * (1 - e.deltaY * 0.002))));
    }
  };

  // Clean up any lingering drag listeners on unmount
  useEffect(() => {
    return () => {
      if (dragHandlersRef.current) {
        document.removeEventListener("mousemove", dragHandlersRef.current.onMove);
        document.removeEventListener("mouseup", dragHandlersRef.current.onUp);
        dragHandlersRef.current = null;
      }
    };
  }, []);

  // Playhead drag — capture pxPerSec at drag start for consistency
  const handlePlayheadDown = (e: React.MouseEvent) => {
    e.preventDefault(); e.stopPropagation();
    setDragging(true);
    const pps = pxPerSec; // capture current zoom level
    const dur = outputDuration;
    const onMove = (me: MouseEvent) => {
      if (!timelineRef.current) return;
      const rect = timelineRef.current.getBoundingClientRect();
      const x = me.clientX - rect.left + timelineRef.current.scrollLeft;
      seekToOutput(Math.max(0, Math.min(dur, x / pps)));
    };
    const onUp = () => { setDragging(false); document.removeEventListener("mousemove", onMove); document.removeEventListener("mouseup", onUp); dragHandlersRef.current = null; };
    dragHandlersRef.current = { onMove, onUp };
    document.addEventListener("mousemove", onMove); document.addEventListener("mouseup", onUp);
  };

  // Time ruler ticks based on OUTPUT duration
  const tickInterval = pxPerSec > 20 ? 1 : pxPerSec > 5 ? 5 : pxPerSec > 2 ? 10 : pxPerSec > 0.5 ? 30 : 60;
  const ticks: number[] = [];
  for (let t = 0; t <= outputDuration; t += tickInterval) ticks.push(t);

  const playheadLeft = outputTime * pxPerSec;

  return (
    <div style={{ height: "100%", display: "flex", flexDirection: "column" }}>
      {/* VIDEO PLAYER */}
      <div style={{ borderRadius: 8, overflow: "hidden", background: "#000", flex: 1, minHeight: 100 }}>
        <video ref={videoRef} playsInline style={{ width: "100%", height: "100%", objectFit: "contain", display: src ? "block" : "none" }} />
        {!src && (
          <div style={{ width: "100%", height: "100%", display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", color: C.muted, gap: 8 }}>
            <svg width="36" height="36" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" style={{ opacity: 0.5, marginBottom: 4 }}>
              <rect x="2" y="2" width="20" height="20" rx="2.18"/><line x1="7" y1="2" x2="7" y2="22"/><line x1="17" y1="2" x2="17" y2="22"/><line x1="2" y1="12" x2="22" y2="12"/>
            </svg>
            <div style={{ fontSize: 15, fontWeight: 600, color: C.dim }}>No video loaded</div>
            <div style={{ fontSize: 13 }}>Select a video file in the Project Setup step to start editing.</div>
          </div>
        )}
      </div>

      {/* TRANSPORT BAR */}
      <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "8px 4px", borderBottom: `1px solid ${C.border}`, flexShrink: 0 }}>
        <button aria-label="Skip back" onClick={() => seekToOutput(Math.max(0, outputTime - 5))} style={{ background: "none", border: "none", color: C.dim, cursor: "pointer", padding: 4, display: "flex" }}>
          <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor"><path d="M19 20L9 12l10-8v16zM7 19V5H5v14h2z"/></svg>
        </button>
        <button aria-label={isPlaying ? "Pause" : "Play"} onClick={togglePlay} style={{ background: "rgba(255,255,255,0.06)", border: "none", color: "#fff", cursor: "pointer", padding: 6, borderRadius: 6, display: "flex" }}>
          {isPlaying
            ? <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="4" width="4" height="16"/><rect x="14" y="4" width="4" height="16"/></svg>
            : <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor"><polygon points="5 3 19 12 5 21 5 3"/></svg>}
        </button>
        <button aria-label="Skip forward" onClick={() => seekToOutput(Math.min(outputDuration, outputTime + 5))} style={{ background: "none", border: "none", color: C.dim, cursor: "pointer", padding: 4, display: "flex" }}>
          <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor"><path d="M5 4l10 8-10 8V4zm12-1v14h2V5h-2z"/></svg>
        </button>
        <span style={{ fontFamily: "monospace", fontSize: 13, color: C.text, fontWeight: 600, minWidth: 110 }}>
          {secondsToTimestamp(outputTime)} / {secondsToTimestamp(outputDuration)}
        </span>
        <div style={{ flex: 1 }} />
        <span style={{ fontSize: 11, color: C.muted }}>
          {clips.length} clip{clips.length !== 1 ? "s" : ""}
        </span>
        {/* Undo/Redo */}
        <button onClick={undo} disabled={!canUndo()} title="Undo (Cmd+Z)" aria-label="Undo" style={{
          background: "none", border: "none", color: canUndo() ? C.dim : C.muted, cursor: canUndo() ? "pointer" : "default",
          padding: 4, display: "flex", opacity: canUndo() ? 1 : 0.3,
        }}>
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><polyline points="1 4 1 10 7 10"/><path d="M3.51 15a9 9 0 102.13-9.36L1 10"/></svg>
        </button>
        <button onClick={redo} disabled={!canRedo()} title="Redo (Cmd+Shift+Z)" aria-label="Redo" style={{
          background: "none", border: "none", color: canRedo() ? C.dim : C.muted, cursor: canRedo() ? "pointer" : "default",
          padding: 4, display: "flex", opacity: canRedo() ? 1 : 0.3,
        }}>
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><polyline points="23 4 23 10 17 10"/><path d="M20.49 15a9 9 0 11-2.13-9.36L23 10"/></svg>
        </button>
        <Button variant="secondary" size="sm"
          disabled={clips.length === 1 && clips[0].speed === 1.0 && !clips[0].skipFrames && Math.abs((clips[0].sourceEnd - clips[0].sourceStart) - videoDuration) < 0.5}
          onClick={() => { useEditStore.getState().reset(); if (videoDuration > 0) initFromVideo(videoDuration); }}>Revert to Original</Button>
      </div>

      {/* RESIZE HANDLE */}
      <div
        onMouseDown={(e) => {
          e.preventDefault();
          setResizingTimeline(true);
          const startY = e.clientY;
          const startH = timelineHeight;
          const onMove = (me: MouseEvent) => setTimelineHeight(Math.max(90, Math.min(400, startH - (me.clientY - startY))));
          const onUp = () => { setResizingTimeline(false); document.removeEventListener("mousemove", onMove); document.removeEventListener("mouseup", onUp); };
          document.addEventListener("mousemove", onMove); document.addEventListener("mouseup", onUp);
        }}
        style={{
          height: 6, cursor: "row-resize", flexShrink: 0,
          background: resizingTimeline ? "rgba(99,102,241,0.3)" : "transparent",
          display: "flex", alignItems: "center", justifyContent: "center",
          transition: resizingTimeline ? "none" : "background 0.15s",
        }}
        onMouseEnter={(e) => { if (!resizingTimeline) e.currentTarget.style.background = "rgba(255,255,255,0.06)"; }}
        onMouseLeave={(e) => { if (!resizingTimeline) e.currentTarget.style.background = "transparent"; }}
      >
        <div style={{ width: 32, height: 2, borderRadius: 1, background: "rgba(255,255,255,0.15)" }} />
      </div>

      {/* TIMELINE */}
      <div style={{ height: timelineHeight, minHeight: 90, display: "flex", flexDirection: "column", flexShrink: 0 }}>
        <div ref={timelineRef} onWheel={handleWheel} onClick={handleTimelineClick}
          style={{ flex: 1, overflowX: "auto", overflowY: "hidden", position: "relative", cursor: "crosshair", background: "rgba(255,255,255,0.01)" }}>
          <div style={{ width: Math.max(totalPx, timelineWidth), minHeight: "100%", position: "relative" }}>
            {/* Time ruler */}
            <div style={{ height: 22, borderBottom: `1px solid ${C.border}`, position: "relative" }}>
              {ticks.map((t) => (
                <div key={t} style={{ position: "absolute", left: t * pxPerSec, top: 0, height: "100%", borderLeft: "1px solid rgba(255,255,255,0.06)", paddingLeft: 4 }}>
                  <span style={{ fontSize: 9, color: C.muted, fontFamily: "monospace" }}>{secondsToTimestamp(t)}</span>
                </div>
              ))}
            </div>

            {/* Clip blocks — CONTIGUOUS, draggable for reorder */}
            <div style={{ position: "relative", height: 64 }}>
              {(() => {
                let cumPx = 0;
                return clips.map((clip, i) => {
                  const clipOutputDur = (clip.sourceEnd - clip.sourceStart) / clip.speed;
                  const clipPx = clipOutputDur * pxPerSec;
                  const left = cumPx;
                  cumPx += clipPx;
                  const isSel = i === selectedClipIndex;
                  const isDragging = dragClipIdx === i;

                  const startIdx = Math.floor((clip.sourceStart / videoDuration) * thumbs.length);
                  const endIdx = Math.ceil((clip.sourceEnd / videoDuration) * thumbs.length);
                  const clipThumbs = thumbs.slice(startIdx, Math.max(startIdx + 1, endIdx));

                  return (
                    <div key={clip.id}
                      draggable
                      onDragStart={(e) => { setDragClipIdx(i); e.dataTransfer.effectAllowed = "move"; }}
                      onDragEnd={() => setDragClipIdx(null)}
                      onDragOver={(e) => { e.preventDefault(); e.dataTransfer.dropEffect = "move"; }}
                      onDrop={(e) => { e.preventDefault(); if (dragClipIdx !== null && dragClipIdx !== i) moveClip(dragClipIdx, i); setDragClipIdx(null); }}
                      style={{
                        position: "absolute", top: 2, height: 60, left, width: Math.max(clipPx, 3),
                        borderRadius: 4, overflow: "hidden",
                        pointerEvents: "auto", cursor: "grab",
                        border: isSel ? "2px solid #6366f1" : isDragging ? "2px dashed #6366f1" : "1px solid rgba(255,255,255,0.1)",
                        opacity: isDragging ? 0.5 : 1,
                        display: "flex", transition: "opacity 0.15s",
                      }}>
                      {clipThumbs.map((t, j) => (
                        <div key={j} style={{ flex: 1, backgroundImage: `url(${t})`, backgroundSize: "cover", backgroundPosition: "center", minWidth: 1, pointerEvents: "none" }} />
                      ))}
                      {clip.speed !== 1.0 && (
                        <div style={{ position: "absolute", bottom: 2, left: 4, pointerEvents: "none" }}>
                          <span style={{ fontSize: 8, padding: "1px 5px", borderRadius: 3, background: clip.skipFrames ? "rgba(245,158,11,0.85)" : "rgba(168,85,247,0.85)", color: "#fff", fontWeight: 700 }}>
                            {clip.speed}x{clip.skipFrames ? " TL" : ""}
                          </span>
                        </div>
                      )}
                      {/* Clip number */}
                      <div style={{ position: "absolute", top: 2, left: 4, fontSize: 9, color: "rgba(255,255,255,0.6)", fontWeight: 600, pointerEvents: "none", textShadow: "0 1px 2px rgba(0,0,0,0.5)" }}>
                        {i + 1}
                      </div>
                    </div>
                  );
                });
              })()}

              {/* Add media button at end of timeline */}
              {(() => {
                let endPx = 0;
                clips.forEach((c) => { endPx += ((c.sourceEnd - c.sourceStart) / c.speed) * pxPerSec; });
                return (
                  <div
                    onClick={async (e) => {
                      e.stopPropagation();
                      const { open } = await import("@tauri-apps/plugin-dialog");
                      const file = await open({ multiple: false, filters: [{ name: "Media", extensions: ["mp4", "mov", "avi", "mkv", "webm", "jpg", "jpeg", "png", "gif"] }] });
                      if (!file) return;
                      try {
                        const { probeVideo } = await import("../../lib/tauri/commands");
                        const m = await probeVideo(file as string);
                        // For now, add as a clip from the same source video at the end
                        // TODO: multi-source support requires backend changes
                        // For images, this won't work yet - just videos from same source
                        useEditStore.getState().addClip(file as string, 0, m.duration_seconds);
                      } catch (err) { console.error("Failed to add media:", err); }
                    }}
                    style={{
                      position: "absolute", top: 2, height: 60, left: endPx, width: 48,
                      borderRadius: 4, cursor: "pointer", pointerEvents: "auto",
                      border: "2px dashed rgba(255,255,255,0.1)", background: "rgba(255,255,255,0.02)",
                      display: "flex", alignItems: "center", justifyContent: "center",
                      flexDirection: "column", gap: 2, transition: "all 0.12s",
                    }}
                    onMouseEnter={(e) => { e.currentTarget.style.borderColor = "rgba(99,102,241,0.4)"; e.currentTarget.style.background = "rgba(99,102,241,0.06)"; }}
                    onMouseLeave={(e) => { e.currentTarget.style.borderColor = "rgba(255,255,255,0.1)"; e.currentTarget.style.background = "rgba(255,255,255,0.02)"; }}
                  >
                    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="#6366f1" strokeWidth="2" strokeLinecap="round"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
                  </div>
                );
              })()}
            </div>

            {/* Playhead */}
            <div onMouseDown={handlePlayheadDown} style={{
              position: "absolute", top: 0, bottom: 0, left: playheadLeft,
              width: 2, background: "#ef4444", zIndex: 10, cursor: "col-resize",
              boxShadow: "0 0 6px rgba(239,68,68,0.4)", pointerEvents: "auto",
            }}>
              <div style={{ position: "absolute", top: 0, left: -5, width: 12, height: 12, background: "#ef4444", borderRadius: "2px 2px 50% 50%", clipPath: "polygon(0 0, 100% 0, 100% 60%, 50% 100%, 0 60%)" }} />
            </div>
          </div>
        </div>

        {/* CLIP INSPECTOR */}
        <div style={{ flexShrink: 0, padding: "8px 12px", borderTop: `1px solid ${C.border}`, display: "flex", alignItems: "center", gap: 12, minHeight: 44 }}>
          {selClip ? (
            <>
              <span style={{ fontSize: 12, color: C.dim, fontWeight: 600 }}>
                Clip {(selectedClipIndex ?? 0) + 1}/{clips.length}
              </span>
              <span style={{ fontSize: 11, color: C.muted }}>
                src {secondsToTimestamp(selClip.sourceStart)}→{secondsToTimestamp(selClip.sourceEnd)} &middot; {((selClip.sourceEnd - selClip.sourceStart) / selClip.speed).toFixed(1)}s out
              </span>
              <div style={{ width: 1, height: 20, background: C.border }} />
              <Button variant="secondary" size="sm" onClick={() => splitAt(outputTime)}>Split (S)</Button>
              <Button variant="danger" size="sm" onClick={() => { if (selectedClipIndex !== null && clips.length > 1) deleteClip(selectedClipIndex); }} disabled={clips.length <= 1}>Delete</Button>
              <div style={{ width: 1, height: 20, background: C.border }} />
              <span style={{ fontSize: 11, color: C.muted }}>Speed</span>
              <input type="range" min="0.25" max="10" step="0.25" value={selClip.speed}
                onChange={(e) => setClipSpeedLive(selectedClipIndex!, parseFloat(e.target.value))}
                onMouseUp={() => commitSpeedChange()}
                onTouchEnd={() => commitSpeedChange()}
                style={{ width: 80, accentColor: "#6366f1" }} />
              <input type="number" min="0.25" max="10" step="0.25" value={selClip.speed}
                onChange={(e) => { const v = parseFloat(e.target.value); if (v >= 0.25 && v <= 10) setClipSpeed(selectedClipIndex!, v); }}
                style={{ width: 38, padding: "2px 4px", borderRadius: 4, border: `1px solid ${C.border}`, background: "rgba(255,255,255,0.04)", color: selClip.speed !== 1 ? "#a855f7" : C.dim, fontSize: 11, fontWeight: 600, textAlign: "center", outline: "none", fontFamily: "inherit" }} />
              <span style={{ fontSize: 11, color: C.muted }}>x</span>
              {selClip.speed > 1 && (
                <>
                  <span style={{ fontSize: 10, color: C.muted, marginLeft: 2 }}>
                    ({((selClip.sourceEnd - selClip.sourceStart)).toFixed(0)}s→{((selClip.sourceEnd - selClip.sourceStart) / selClip.speed).toFixed(0)}s)
                  </span>
                  <div style={{ width: 1, height: 20, background: C.border }} />
                  {/* Speed mode vs Skip Frames toggle */}
                  <div style={{ display: "flex", gap: 2, background: "rgba(255,255,255,0.03)", borderRadius: 5, padding: 2 }}>
                    <button onClick={() => setClipSkipFrames(selectedClipIndex!, false)} title="Plays all frames faster (fast-forward effect)" style={{
                      padding: "3px 8px", borderRadius: 4, border: "none", fontSize: 10, fontWeight: 600,
                      background: !selClip.skipFrames ? "rgba(168,85,247,0.2)" : "transparent",
                      color: !selClip.skipFrames ? "#a855f7" : C.muted,
                      cursor: "pointer", fontFamily: "inherit",
                    }}>Speed up</button>
                    <button onClick={() => setClipSkipFrames(selectedClipIndex!, true)} title="Drops frames for clean jump-cuts (no jitter)" style={{
                      padding: "3px 8px", borderRadius: 4, border: "none", fontSize: 10, fontWeight: 600,
                      background: selClip.skipFrames ? "rgba(245,158,11,0.2)" : "transparent",
                      color: selClip.skipFrames ? "#f59e0b" : C.muted,
                      cursor: "pointer", fontFamily: "inherit",
                    }}>Time-lapse</button>
                  </div>
                </>
              )}
            </>
          ) : (
            <span style={{ fontSize: 12, color: C.muted }}>Click timeline to place playhead. Press S to split. Ctrl+Scroll to zoom.</span>
          )}
        </div>
      </div>
    </div>
  );
}
