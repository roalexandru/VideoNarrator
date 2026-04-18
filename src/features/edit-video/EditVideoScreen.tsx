import { useRef, useEffect, useState, useCallback } from "react";
import { useProjectStore } from "../../stores/projectStore";
import { useEditStore, clipOutputDuration, EFFECT_META } from "../../stores/editStore";
import type { EasingPreset, ZoomPanEffect, EffectType, TimelineEffect } from "../../stores/editStore";
import { extractEditThumbnails } from "../../lib/tauri/commands";
import { Button } from "../../components/ui/Button";
import { secondsToTimestamp } from "../../lib/formatters";
import { convertFileSrc } from "@tauri-apps/api/core";
import { trackError } from "../telemetry/analytics";
import { computeZoomTransform, computeZoomAtTime } from "./easing";
import { ZoomPanOverlay } from "./ZoomPanOverlay";
import { EffectsOverlay } from "./EffectsOverlay";
import { EffectInspector } from "./EffectInspector";
import { NumericInput } from "./NumericInput";

const C = { text: "#e0e0ea", dim: "#8b8ba0", muted: "#5a5a6e", border: "rgba(255,255,255,0.07)", accent: "#818cf8" };

const DEFAULT_ZOOM_PAN: ZoomPanEffect = {
  startRegion: { x: 0, y: 0, width: 1, height: 1 },
  endRegion: { x: 0.25, y: 0.25, width: 0.5, height: 0.5 },
  easing: 'ease-in-out',
};

export function EditVideoScreen() {
  const videoFile = useProjectStore((s) => s.videoFile);
  const projectId = useProjectStore((s) => s.projectId);
  const store = useEditStore();
  const { clips, effects, selectedClipIndex, selectedEffectId, initFromVideo, splitAt, deleteClip, setClipSpeed, setClipSpeedLive, commitSpeedChange, setClipSkipFrames, moveClip, selectClip, undo, redo, canUndo, canRedo, insertFreezeFrame, setFreezeDuration, setClipZoomPanLive, commitZoomPanChange, addEffect, removeEffect, updateEffect, updateEffectLive, commitEffectChange, selectEffect } = store;

  const videoRef = useRef<HTMLVideoElement>(null);
  const timelineRef = useRef<HTMLDivElement>(null);
  const dragHandlersRef = useRef<{ onMove: (e: MouseEvent) => void; onUp: () => void } | null>(null);
  const rafRef = useRef<number>(0);
  const outputTimeRef = useRef(0); // mirrors outputTime for use in rAF tick without stale closures
  const isUserPlayingRef = useRef(false); // true when user wants playback (survives freeze pauses)
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
  const [isEditingZoomPan, setIsEditingZoomPan] = useState(false);
  const [activeZoomRegion, setActiveZoomRegion] = useState<'start' | 'end'>('start');
  const [showAddEffectMenu, setShowAddEffectMenu] = useState(false);
  const [videoContainerRect, setVideoContainerRect] = useState({ width: 0, height: 0, left: 0, top: 0 });
  const videoContainerRef = useRef<HTMLDivElement>(null);

  // Persist timeline height
  useEffect(() => { try { localStorage.setItem("narrator_timeline_height", String(timelineHeight)); } catch { /* storage full or unavailable */ } }, [timelineHeight]);

  // Keep outputTimeRef in sync for rAF tick access without stale closures
  useEffect(() => { outputTimeRef.current = outputTime; }, [outputTime]);

  const src = videoFile?.path ? convertFileSrc(videoFile.path) : undefined;
  const selClip = selectedClipIndex !== null ? clips[selectedClipIndex] : null;
  const outputDuration = store.getOutputDuration();

  // Debug state — visible in dev builds only, toggled via the "D" key.
  // Surfaces <video> element diagnostics since WebView devtools is clunky.
  const [debugPanel, setDebugPanel] = useState(() => {
    try { return localStorage.getItem("narrator_debug_panel") === "1"; } catch { return false; }
  });
  const [videoDiag, setVideoDiag] = useState<{ readyState: number; networkState: number; videoWidth: number; videoHeight: number; currentSrc: string; error: string | null; paused: boolean } | null>(null);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key.toLowerCase() === "d" && (e.ctrlKey || e.metaKey) && e.shiftKey) {
        e.preventDefault();
        setDebugPanel((d) => {
          const next = !d;
          try { localStorage.setItem("narrator_debug_panel", next ? "1" : "0"); } catch { /* empty */ }
          return next;
        });
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  useEffect(() => {
    if (!debugPanel) return;
    const id = setInterval(() => {
      const v = videoRef.current;
      if (!v) return;
      setVideoDiag({
        readyState: v.readyState,
        networkState: v.networkState,
        videoWidth: v.videoWidth,
        videoHeight: v.videoHeight,
        currentSrc: v.currentSrc || "",
        error: v.error ? `code=${v.error.code} ${v.error.message}` : null,
        paused: v.paused,
      });
    }, 250);
    return () => clearInterval(id);
  }, [debugPanel]);

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
        const dur = clipOutputDuration(clip);
        if (clip.type === 'freeze') {
          // Freeze clips are driven by outputTime state, not video time
          cumOut += dur;
          continue;
        }
        if (sourceT >= clip.sourceStart && sourceT <= clip.sourceEnd) {
          cumOut += (sourceT - clip.sourceStart) / clip.speed;
          setOutputTime(cumOut);
          return;
        }
        cumOut += dur;
      }
      setOutputTime(cumOut);
    };
    const onPlay = () => setIsPlaying(true);
    const onPause = () => {
      // Only set isPlaying=false if the user actually stopped playback.
      // During freeze clips, we pause the video but keep the rAF loop running.
      if (!isUserPlayingRef.current) setIsPlaying(false);
    };
    v.addEventListener("loadedmetadata", onMeta); v.addEventListener("timeupdate", onTime);
    v.addEventListener("play", onPlay); v.addEventListener("pause", onPause);
    return () => { v.removeEventListener("loadedmetadata", onMeta); v.removeEventListener("timeupdate", onTime); v.removeEventListener("play", onPlay); v.removeEventListener("pause", onPause); };
  }, [dragging, clips]);

  const [thumbsLoading, setThumbsLoading] = useState(false);

  // Extract thumbnails
  useEffect(() => {
    if (!videoFile?.path) return;
    setThumbsLoading(true);
    const dur = videoFile.duration || 120;
    const thumbCount = dur > 600 ? 30 : dur > 300 ? 40 : 60;
    extractEditThumbnails(videoFile.path, `/tmp/narrator_edit_thumbs_${projectId || crypto.randomUUID()}`, thumbCount)
      .then((paths) => setThumbs(paths.map((p) => convertFileSrc(p))))
      .catch(() => {})
      .finally(() => setThumbsLoading(false));
  }, [videoFile?.path, projectId]);

  // Track video container size for zoom/pan overlay
  // Compute the actual rendered video rect within the container (objectFit: "contain" causes letterboxing)
  useEffect(() => {
    const el = videoContainerRef.current;
    if (!el) return;
    const update = () => {
      const containerRect = el.getBoundingClientRect();
      const v = videoRef.current;
      const videoW = v?.videoWidth || videoFile?.resolution?.width || 0;
      const videoH = v?.videoHeight || videoFile?.resolution?.height || 0;
      if (videoW > 0 && videoH > 0 && containerRect.width > 0 && containerRect.height > 0) {
        // Compute the actual rendered video area within the container (objectFit: "contain")
        const containerAspect = containerRect.width / containerRect.height;
        const videoAspect = videoW / videoH;
        let renderedW: number, renderedH: number;
        if (videoAspect > containerAspect) {
          // Video is wider — letterboxed top/bottom
          renderedW = containerRect.width;
          renderedH = containerRect.width / videoAspect;
        } else {
          // Video is taller — pillarboxed left/right
          renderedH = containerRect.height;
          renderedW = containerRect.height * videoAspect;
        }
        setVideoContainerRect({ width: renderedW, height: renderedH, left: containerRect.left, top: containerRect.top });
      } else {
        setVideoContainerRect({ width: containerRect.width, height: containerRect.height, left: containerRect.left, top: containerRect.top });
      }
    };
    const observer = new ResizeObserver(update);
    observer.observe(el);
    // Also update when video metadata loads
    const v = videoRef.current;
    if (v) v.addEventListener("loadedmetadata", update);
    return () => { observer.disconnect(); if (v) v.removeEventListener("loadedmetadata", update); };
  }, [videoFile?.resolution?.width, videoFile?.resolution?.height]);

  // Auto-show zoom regions when a zoom effect is selected, hide otherwise
  useEffect(() => {
    const isZoomSelected = selectedEffectId != null && effects.find((e) => e.id === selectedEffectId)?.type === 'zoom-pan';
    setIsEditingZoomPan(!!isZoomSelected);
  }, [selectedEffectId, effects]);

  // Close zoom/pan editing when clip changes
  useEffect(() => {
    setIsEditingZoomPan(false);
  }, [selectedClipIndex]);

  // Compute current zoom transform for CSS preview (effects track takes priority, falls back to clip.zoomPan)
  const selectedEffect = selectedEffectId ? effects.find((e) => e.id === selectedEffectId) : null;
  const zoomTransform = (() => {
    if (videoContainerRect.width <= 0) return { scale: 1, tx: 0, ty: 0 };
    // Check effects track first
    const effectTransform = computeZoomAtTime(effects, outputTime, videoContainerRect.width, videoContainerRect.height);
    if (effectTransform.scale !== 1 || effectTransform.tx !== 0 || effectTransform.ty !== 0) return effectTransform;
    // Fall back to legacy clip.zoomPan
    if (selClip?.zoomPan) {
      let cumOut = 0;
      for (let i = 0; i < clips.length; i++) {
        const dur = clipOutputDuration(clips[i]);
        if (i === selectedClipIndex && outputTime >= cumOut && outputTime <= cumOut + dur) {
          const localTime = outputTime - cumOut;
          return computeZoomTransform(clips[i], localTime, videoContainerRect.width, videoContainerRect.height);
        }
        cumOut += dur;
      }
    }
    return { scale: 1, tx: 0, ty: 0 };
  })();

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
        const d = clipOutputDuration(clip);
        if (clamped < cum + d) {
          if (clip.type === 'freeze') {
            // Don't set playbackRate=0 (unsupported in many browsers). Tick handler pauses video for freeze clips.
            videoRef.current.playbackRate = 1;
          } else {
            videoRef.current.playbackRate = clip.speed;
          }
          break;
        }
        cum += d;
      }
    }
  }, [outputDuration, store, clips]);

  // During playback: handle clip transitions (skip gaps, change speed at boundaries, freeze clips)
  // Uses isUserPlayingRef + outputTimeRef to avoid stale closures.
  // The rAF loop runs as long as isUserPlayingRef is true, even when the video is paused for freeze clips.
  useEffect(() => {
    if (!isPlaying || !videoRef.current) return;
    const v = videoRef.current;
    const tick = () => {
      if (!isUserPlayingRef.current) return; // user stopped playback
      const curOutputTime = outputTimeRef.current;
      let cum = 0;
      for (let i = 0; i < clips.length; i++) {
        const clip = clips[i];
        const dur = clipOutputDuration(clip);

        if (curOutputTime >= cum && curOutputTime < cum + dur) {
          if (clip.type === 'freeze') {
            // Freeze clips: pause the video element, hold at freezeSourceTime, advance output time manually
            const freezeT = clip.freezeSourceTime ?? clip.sourceStart;
            if (!v.paused) v.pause();
            if (Math.abs(v.currentTime - freezeT) > 0.1) {
              v.currentTime = freezeT;
            }
            setOutputTime((prev) => {
              const next = prev + (1 / 60);
              if (next >= cum + dur) {
                // Transition to next clip
                if (i < clips.length - 1) {
                  const nextClip = clips[i + 1];
                  if (nextClip.type === 'freeze') {
                    v.currentTime = nextClip.freezeSourceTime ?? nextClip.sourceStart;
                  } else {
                    v.currentTime = nextClip.sourceStart;
                    v.playbackRate = nextClip.speed;
                    v.play(); // resume video playback after freeze
                  }
                } else {
                  isUserPlayingRef.current = false;
                  setIsPlaying(false);
                }
                return Math.min(next, cum + dur);
              }
              return next;
            });
          } else {
            // Normal clip: ensure video is playing and at correct speed
            if (v.paused && isUserPlayingRef.current) v.play();
            if (Math.abs(v.playbackRate - clip.speed) > 0.01) {
              v.playbackRate = clip.speed;
            }
            // If we've reached the end of this clip, jump to next
            const sourceT = v.currentTime;
            if (sourceT >= clip.sourceEnd - 0.05 && i < clips.length - 1) {
              const nextClip = clips[i + 1];
              if (nextClip.type === 'freeze') {
                v.pause(); // pause for incoming freeze
                v.currentTime = nextClip.freezeSourceTime ?? nextClip.sourceStart;
              } else {
                v.currentTime = nextClip.sourceStart;
                v.playbackRate = nextClip.speed;
              }
            }
          }
          break;
        }
        cum += dur;
      }
      // If past the last clip, stop
      if (clips.length > 0) {
        const totalDur = store.getOutputDuration();
        if (curOutputTime >= totalDur - 0.05) {
          isUserPlayingRef.current = false;
          setIsPlaying(false);
          v.pause();
          return; // don't schedule next tick
        }
      }
      if (isUserPlayingRef.current) {
        rafRef.current = requestAnimationFrame(tick);
      }
    };
    rafRef.current = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(rafRef.current);
  }, [isPlaying, clips, store]);

  // Keyboard
  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement).tagName;
      if (tag === "INPUT" || tag === "TEXTAREA") return;
      if (e.code === "Space") { e.preventDefault(); togglePlay(); }
      if (e.code === "KeyS") { e.preventDefault(); splitAt(outputTime); }
      if (e.code === "KeyF") { e.preventDefault(); insertFreezeFrame(outputTime); }
      if (e.code === "Escape") {
        e.preventDefault();
        if (showAddEffectMenu) setShowAddEffectMenu(false);
        else if (isEditingZoomPan) setIsEditingZoomPan(false);
        else if (selectedEffectId) selectEffect(null);
        return;
      }
      if (e.code === "Delete" || e.code === "Backspace") {
        e.preventDefault();
        // Delete selected effect first, then clip
        if (selectedEffectId) { removeEffect(selectedEffectId); }
        else if (selectedClipIndex !== null && clips.length > 1) { deleteClip(selectedClipIndex); }
        return;
      }
      if (e.code === "ArrowLeft") { e.preventDefault(); seekToOutput(outputTime - 1); }
      if (e.code === "ArrowRight") { e.preventDefault(); seekToOutput(outputTime + 1); }
      if ((e.metaKey || e.ctrlKey) && e.code === "KeyZ" && !e.shiftKey) { e.preventDefault(); undo(); }
      if ((e.metaKey || e.ctrlKey) && e.code === "KeyZ" && e.shiftKey) { e.preventDefault(); redo(); }
    };
    window.addEventListener("keydown", h); return () => window.removeEventListener("keydown", h);
  }, [isPlaying, selectedClipIndex, selectedEffectId, outputTime, clips, seekToOutput, splitAt, deleteClip, undo, redo, insertFreezeFrame, isEditingZoomPan, showAddEffectMenu, selectEffect, removeEffect]);

  const togglePlay = () => {
    const v = videoRef.current; if (!v) return;
    if (isPlaying || isUserPlayingRef.current) {
      // User stops playback
      isUserPlayingRef.current = false;
      setIsPlaying(false);
      v.pause();
    } else {
      // User starts playback
      isUserPlayingRef.current = true;
      setIsPlaying(true);
      // Check if we're in a freeze clip — don't start video playback if so
      let cum = 0;
      for (const clip of clips) {
        const d = clipOutputDuration(clip);
        if (outputTime >= cum && outputTime < cum + d) {
          if (clip.type === 'freeze') {
            // Don't call v.play() — the rAF tick will handle freeze advancement
            return;
          }
          v.playbackRate = clip.speed;
          break;
        }
        cum += d;
      }
      v.play().then(() => {
        // Re-enforce playbackRate after play resolves (some browsers reset it)
        let c2 = 0;
        for (const clip of clips) {
          const d2 = clipOutputDuration(clip);
          if (outputTime >= c2 && outputTime < c2 + d2 && clip.type !== 'freeze') {
            v.playbackRate = clip.speed;
            break;
          }
          c2 += d2;
        }
      }).catch(() => {});
    }
  };

  // Timeline layout: clips are contiguous, no gaps
  const timelineWidth = timelineRef.current?.clientWidth || 800;
  const pxPerSec = zoom > 0 ? zoom : (outputDuration > 0 ? timelineWidth / outputDuration : 1);
  const totalPx = outputDuration * pxPerSec;

  // Click timeline → seek. Use pxPerSec directly, not DOM measurements.
  const handleTimelineClick = (e: React.MouseEvent) => {
    if (!timelineRef.current || outputDuration <= 0 || dragging) return;
    const rect = timelineRef.current.getBoundingClientRect();
    const x = e.clientX - rect.left + timelineRef.current.scrollLeft;
    const t = x / pxPerSec;
    seekToOutput(Math.max(0, Math.min(outputDuration, t)));
    // Deselect effect when clicking the timeline background
    if (selectedEffectId) selectEffect(null);
    let cum = 0;
    for (let i = 0; i < clips.length; i++) {
      const d = clipOutputDuration(clips[i]);
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

  // Effect drag — uses document-level listeners to avoid losing drag on mouse-leave
  const handleEffectDragStart = useCallback((e: React.MouseEvent, effectId: string, edge: 'start' | 'end' | 'move', origStart: number, origEnd: number) => {
    e.preventDefault(); e.stopPropagation();
    selectEffect(effectId);
    const startX = e.clientX;
    const pps = pxPerSec;
    const onMove = (me: MouseEvent) => {
      const timeDelta = (me.clientX - startX) / pps;
      if (edge === 'move') {
        const newStart = Math.max(0, origStart + timeDelta);
        const dur = origEnd - origStart;
        updateEffectLive(effectId, { startTime: newStart, endTime: Math.min(newStart + dur, outputDuration) });
      } else if (edge === 'start') {
        const newStart = Math.max(0, Math.min(origEnd - 0.5, origStart + timeDelta));
        updateEffectLive(effectId, { startTime: newStart });
      } else {
        const newEnd = Math.min(outputDuration, Math.max(origStart + 0.5, origEnd + timeDelta));
        updateEffectLive(effectId, { endTime: newEnd });
      }
    };
    const onUp = () => {
      commitEffectChange();
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
    };
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
  }, [pxPerSec, outputDuration, selectEffect, updateEffectLive, commitEffectChange]);

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
      <div ref={videoContainerRef} onClick={(e) => { if (selectedEffectId && e.target === e.currentTarget) selectEffect(null); }} style={{ borderRadius: 8, overflow: "hidden", background: "#000", flex: 1, minHeight: 100, position: "relative" }}>
        <div style={{
          width: "100%", height: "100%",
          overflow: "hidden",
          transform: (isEditingZoomPan || selectedEffect?.type === 'zoom-pan')
            ? undefined  // never apply zoom transform while editing zoom regions
            : (zoomTransform.scale !== 1 || zoomTransform.tx !== 0 || zoomTransform.ty !== 0)
              ? `translate(${zoomTransform.tx}px, ${zoomTransform.ty}px) scale(${zoomTransform.scale})`
              : undefined,
          transformOrigin: "0 0",
          willChange: selClip?.zoomPan || effects.length > 0 ? "transform" : undefined,
          transition: isPlaying ? "none" : "transform 0.15s ease-out",
        }}>
          <video ref={videoRef} playsInline style={{ width: "100%", height: "100%", objectFit: "contain", display: src ? "block" : "none" }} />
        </div>
        {debugPanel && (
          <div style={{
            position: "absolute", top: 8, left: 8, zIndex: 20,
            background: "rgba(0,0,0,0.85)", color: "#0f0",
            fontFamily: "ui-monospace, SFMono-Regular, monospace",
            fontSize: 11, padding: "8px 10px", borderRadius: 6,
            maxWidth: 520, lineHeight: 1.5, pointerEvents: "none",
            border: "1px solid #0f0",
          }}>
            <div style={{ color: "#ff0", fontWeight: 700, marginBottom: 4 }}>
              VIDEO DEBUG (⌘⇧D to toggle)
            </div>
            <div>src: <span style={{ color: "#9ff" }}>{src?.slice(0, 80) || "(none)"}</span></div>
            <div>videoFile.path: <span style={{ color: "#9ff" }}>{videoFile?.path?.slice(0, 80) || "(none)"}</span></div>
            {videoDiag && (
              <>
                <div>readyState: <span style={{ color: videoDiag.readyState >= 2 ? "#0f0" : "#f55" }}>{videoDiag.readyState}</span> (want ≥2)</div>
                <div>networkState: {videoDiag.networkState}</div>
                <div>videoWidth × videoHeight: <span style={{ color: videoDiag.videoWidth > 0 ? "#0f0" : "#f55" }}>{videoDiag.videoWidth} × {videoDiag.videoHeight}</span></div>
                <div>paused: {String(videoDiag.paused)}</div>
                <div>currentSrc: <span style={{ color: "#9ff" }}>{videoDiag.currentSrc.slice(0, 80) || "(none)"}</span></div>
                {videoDiag.error && <div style={{ color: "#f55" }}>ERROR: {videoDiag.error}</div>}
              </>
            )}
            <div style={{ marginTop: 6, borderTop: "1px dashed #0f0", paddingTop: 6 }}>
              transform.scale: {zoomTransform.scale.toFixed(3)}
            </div>
            <div>transform.tx: {zoomTransform.tx.toFixed(1)}  ty: {zoomTransform.ty.toFixed(1)}</div>
            <div>effects: {effects.length} (clip-level zoomPan: {selClip?.zoomPan ? "yes" : "no"})</div>
          </div>
        )}
        {!src && (
          <div style={{ position: "absolute", inset: 0, display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", color: C.muted, gap: 8 }}>
            <svg width="36" height="36" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" style={{ opacity: 0.5, marginBottom: 4 }}>
              <rect x="2" y="2" width="20" height="20" rx="2.18"/><line x1="7" y1="2" x2="7" y2="22"/><line x1="17" y1="2" x2="17" y2="22"/><line x1="2" y1="12" x2="22" y2="12"/>
            </svg>
            <div style={{ fontSize: 15, fontWeight: 600, color: C.dim }}>No video loaded</div>
            <div style={{ fontSize: 13 }}>Select a video file in the Project Setup step to start editing.</div>
          </div>
        )}
        {/* Effects overlay — spotlight, blur, text, fade with interactive handles */}
        {videoContainerRect.width > 0 && (
          <EffectsOverlay
            effects={effects}
            outputTime={outputTime}
            videoWidth={videoContainerRect.width}
            videoHeight={videoContainerRect.height}
            selectedEffectId={selectedEffectId}
            onUpdateEffect={updateEffect}
            onUpdateEffectLive={updateEffectLive}
            onCommitEffect={commitEffectChange}
          />
        )}
        {/* Zoom/Pan overlay for region editing — supports both clip-level and effect-level */}
        {isEditingZoomPan && videoContainerRect.width > 0 && (() => {
          // Determine which zoom data to edit: effect takes priority, then clip
          const editingEffect = selectedEffect?.zoomPan ? selectedEffect : null;
          const editingClip = !editingEffect && selClip?.zoomPan && selectedClipIndex !== null ? selClip : null;
          const zp = editingEffect?.zoomPan ?? editingClip?.zoomPan ?? null;
          if (!zp) return null;
          return (
            <ZoomPanOverlay
              videoRect={videoContainerRect}
              startRegion={zp.startRegion}
              endRegion={zp.endRegion}
              activeRegion={activeZoomRegion}
              onActiveRegionChange={setActiveZoomRegion}
              onStartChange={(r) => {
                if (editingEffect) {
                  updateEffectLive(editingEffect.id, { zoomPan: { ...editingEffect.zoomPan!, startRegion: r } });
                } else if (editingClip && selectedClipIndex !== null) {
                  setClipZoomPanLive(selectedClipIndex, { ...editingClip.zoomPan!, startRegion: r });
                }
              }}
              onEndChange={(r) => {
                if (editingEffect) {
                  updateEffectLive(editingEffect.id, { zoomPan: { ...editingEffect.zoomPan!, endRegion: r } });
                } else if (editingClip && selectedClipIndex !== null) {
                  setClipZoomPanLive(selectedClipIndex, { ...editingClip.zoomPan!, endRegion: r });
                }
              }}
              onCommit={() => editingEffect ? commitEffectChange() : commitZoomPanChange()}
            />
          );
        })()}
      </div>

      {/* TRANSPORT BAR */}
      <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "8px 4px", borderBottom: `1px solid ${C.border}`, flexShrink: 0, opacity: src ? 1 : 0.3, pointerEvents: src ? "auto" : "none" }}>
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

            {/* Thumbnail loading shimmer */}
            {thumbsLoading && (
              <div style={{ position: "absolute", top: 24, left: 0, right: 0, height: 62, background: "linear-gradient(90deg, rgba(255,255,255,0.02) 0%, rgba(255,255,255,0.06) 50%, rgba(255,255,255,0.02) 100%)", backgroundSize: "200% 100%", animation: "shimmer 1.5s infinite", zIndex: 0, borderRadius: 4 }} />
            )}
            {/* Clip blocks — CONTIGUOUS, draggable for reorder */}
            <div style={{ position: "relative", height: 64 }}>
              {(() => {
                let cumPx = 0;
                return clips.map((clip, i) => {
                  const clipOutputDur = clipOutputDuration(clip);
                  const clipPx = clipOutputDur * pxPerSec;
                  const left = cumPx;
                  cumPx += clipPx;
                  const isSel = i === selectedClipIndex;
                  const isDragging = dragClipIdx === i;

                  const isFreeze = clip.type === 'freeze';
                  const hasZoom = !!clip.zoomPan;
                  const freezeThumbIdx = isFreeze ? Math.floor(((clip.freezeSourceTime ?? 0) / videoDuration) * thumbs.length) : 0;
                  const startIdx = isFreeze ? freezeThumbIdx : Math.floor((clip.sourceStart / videoDuration) * thumbs.length);
                  const endIdx = isFreeze ? freezeThumbIdx + 1 : Math.ceil((clip.sourceEnd / videoDuration) * thumbs.length);
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
                        border: isSel ? `2px solid ${isFreeze ? "#38bdf8" : "#6366f1"}` : isDragging ? "2px dashed #6366f1" : isFreeze ? "1px solid rgba(56,189,248,0.3)" : "1px solid rgba(255,255,255,0.1)",
                        opacity: isDragging ? 0.5 : 1,
                        display: "flex", transition: "opacity 0.15s",
                      }}>
                      {clipThumbs.map((t, j) => (
                        <div key={j} style={{ flex: 1, backgroundImage: `url(${t})`, backgroundSize: "cover", backgroundPosition: "center", minWidth: 1, pointerEvents: "none" }} />
                      ))}
                      {clip.speed !== 1.0 && !isFreeze && (
                        <div style={{ position: "absolute", bottom: 2, left: 4, pointerEvents: "none" }}>
                          <span style={{ fontSize: 8, padding: "1px 5px", borderRadius: 3, background: clip.skipFrames ? "rgba(245,158,11,0.85)" : "rgba(168,85,247,0.85)", color: "#fff", fontWeight: 700 }}>
                            {clip.speed}x{clip.skipFrames ? " TL" : ""}
                          </span>
                        </div>
                      )}
                      {/* Freeze badge */}
                      {isFreeze && (
                        <div style={{ position: "absolute", bottom: 2, left: 4, pointerEvents: "none" }}>
                          <span style={{ fontSize: 8, padding: "1px 5px", borderRadius: 3, background: "rgba(56,189,248,0.85)", color: "#fff", fontWeight: 700 }}>
                            {(clip.freezeDuration ?? 3).toFixed(2)}s
                          </span>
                        </div>
                      )}
                      {/* Zoom/Pan badge */}
                      {hasZoom && (
                        <div style={{ position: "absolute", bottom: 2, right: 4, pointerEvents: "none" }}>
                          <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="#fff" strokeWidth="2.5" strokeLinecap="round">
                            <circle cx="11" cy="11" r="7"/><line x1="16.5" y1="16.5" x2="21" y2="21"/>
                          </svg>
                        </div>
                      )}
                      {/* Clip number / freeze icon */}
                      <div style={{ position: "absolute", top: 2, left: 4, fontSize: 9, color: "rgba(255,255,255,0.6)", fontWeight: 600, pointerEvents: "none", textShadow: "0 1px 2px rgba(0,0,0,0.5)", display: "flex", alignItems: "center", gap: 3 }}>
                        {isFreeze && (
                          <svg width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round">
                            <rect x="6" y="4" width="4" height="16"/><rect x="14" y="4" width="4" height="16"/>
                          </svg>
                        )}
                        {i + 1}
                      </div>
                    </div>
                  );
                });
              })()}

              {/* Add media button at end of timeline */}
              {(() => {
                let endPx = 0;
                clips.forEach((c) => { endPx += clipOutputDuration(c) * pxPerSec; });
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
                      } catch (err) { console.error("Failed to add media:", err); trackError("edit_add_media", err); }
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

            {/* Effects track — stacked lanes for overlapping effects */}
            {(() => {
              // Compute lanes: assign each effect to the lowest lane that doesn't overlap
              const LANE_H = 24;
              const LANE_GAP = 2;
              const lanes: { endTime: number }[] = [];
              const effectLanes: Map<string, number> = new Map();
              const sorted = [...effects].sort((a, b) => a.startTime - b.startTime || a.id.localeCompare(b.id));
              for (const effect of sorted) {
                let lane = 0;
                while (lane < lanes.length && lanes[lane].endTime > effect.startTime) lane++;
                if (lane >= lanes.length) lanes.push({ endTime: effect.endTime });
                else lanes[lane].endTime = Math.max(lanes[lane].endTime, effect.endTime);
                effectLanes.set(effect.id, lane);
              }
              const numLanes = Math.max(1, lanes.length);
              const trackH = numLanes * (LANE_H + LANE_GAP) + 4;

              return (
                <div style={{ position: "relative", height: trackH, borderTop: `1px solid ${C.border}` }}>
                  <div style={{ position: "absolute", top: 2, left: 4, fontSize: 8, color: C.muted, fontWeight: 700, letterSpacing: 0.5, textTransform: "uppercase", pointerEvents: "none", zIndex: 1 }}>FX</div>
                  {effects.map((effect) => {
                    const meta = EFFECT_META[effect.type];
                    const leftPx = effect.startTime * pxPerSec;
                    const widthPx = Math.max((effect.endTime - effect.startTime) * pxPerSec, 12);
                    const lane = effectLanes.get(effect.id) ?? 0;
                    const topPx = 2 + lane * (LANE_H + LANE_GAP);
                    const isSel = effect.id === selectedEffectId;
                    const transIn = effect.transitionIn ?? 0;
                    const transOut = effect.transitionOut ?? 0;
                    const totalDur = effect.endTime - effect.startTime;
                    const transInPct = totalDur > 0 ? (transIn / totalDur) * 100 : 0;
                    const transOutPct = totalDur > 0 ? ((effect.reverse ? transOut : 0) / totalDur) * 100 : 0;
                    return (
                      <div key={effect.id}
                        onClick={(e) => { e.stopPropagation(); selectEffect(effect.id); }}
                        style={{
                          position: "absolute", top: topPx, height: LANE_H, left: leftPx, width: widthPx,
                          borderRadius: 4, cursor: "grab", pointerEvents: "auto",
                          background: isSel ? `${meta.color}40` : `${meta.color}20`,
                          border: isSel ? `1.5px solid ${meta.color}` : `1px solid ${meta.color}50`,
                          display: "flex", alignItems: "center", justifyContent: "center", gap: 3,
                          overflow: "hidden", userSelect: "none",
                        }}
                        onMouseDown={(e) => handleEffectDragStart(e, effect.id, 'move', effect.startTime, effect.endTime)}
                      >
                        {transInPct > 0 && <div style={{ position: "absolute", left: 0, top: 0, bottom: 0, width: `${transInPct}%`, background: `${meta.color}30`, borderRight: `1px dashed ${meta.color}60`, pointerEvents: "none" }} />}
                        {transOutPct > 0 && <div style={{ position: "absolute", right: 0, top: 0, bottom: 0, width: `${transOutPct}%`, background: `${meta.color}30`, borderLeft: `1px dashed ${meta.color}60`, pointerEvents: "none" }} />}
                        <span style={{ fontSize: 8, color: isSel ? meta.color : C.dim, fontWeight: 600, whiteSpace: "nowrap", overflow: "hidden", zIndex: 1, pointerEvents: "none" }}>{meta.label}</span>
                        <div onMouseDown={(e) => handleEffectDragStart(e, effect.id, 'start', effect.startTime, effect.endTime)} style={{ position: "absolute", left: -2, top: 0, bottom: 0, width: 12, cursor: "ew-resize", zIndex: 2 }} />
                        <div onMouseDown={(e) => handleEffectDragStart(e, effect.id, 'end', effect.startTime, effect.endTime)} style={{ position: "absolute", right: -2, top: 0, bottom: 0, width: 12, cursor: "ew-resize", zIndex: 2 }} />
                      </div>
                    );
                  })}
                </div>
              );
            })()}

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

        {/* INSPECTOR — Two-row stable layout */}
        <div style={{ flexShrink: 0, borderTop: `1px solid ${C.border}` }}>
          {/* ROW 1: Identity + transitions (or clip info) | Delete + Add Effect pinned right */}
          <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "6px 12px", minHeight: 36 }}>
            {/* Left content */}
            <div style={{ display: "flex", alignItems: "center", gap: 8, flex: 1, minWidth: 0 }}>
              {selClip ? (
                <>
                  <span style={{ fontSize: 11, color: selClip.type === 'freeze' ? "#38bdf8" : C.dim, fontWeight: 600, flexShrink: 0 }}>
                    {selClip.type === 'freeze' ? 'Freeze' : 'Clip'} {(selectedClipIndex ?? 0) + 1}/{clips.length}
                  </span>
                  {selClip.type === 'freeze' ? (
                    <>
                      <span style={{ fontSize: 10, color: C.muted, flexShrink: 0 }}>at {secondsToTimestamp(selClip.freezeSourceTime ?? 0)}</span>
                      <span style={{ fontSize: 10, color: C.muted, flexShrink: 0 }}>Duration</span>
                      <NumericInput value={selClip.freezeDuration ?? 3} min={0.1} max={30} width={44} color="#38bdf8"
                        onChange={(v) => setFreezeDuration(selectedClipIndex!, v)} />
                      <span style={{ fontSize: 10, color: C.muted }}>s</span>
                    </>
                  ) : (
                    <>
                      <span style={{ fontSize: 10, color: C.muted, flexShrink: 0 }}>
                        {secondsToTimestamp(selClip.sourceStart)}→{secondsToTimestamp(selClip.sourceEnd)} &middot; {clipOutputDuration(selClip).toFixed(2)}s
                      </span>
                      <Button variant="secondary" size="sm" onClick={() => splitAt(outputTime)}>Split (S)</Button>
                      <Button variant="secondary" size="sm" onClick={() => insertFreezeFrame(outputTime)}>Freeze (F)</Button>
                    </>
                  )}
                </>
              ) : selectedEffect ? (() => {
                const eMeta = EFFECT_META[selectedEffect.type];
                const eDur = selectedEffect.endTime - selectedEffect.startTime;
                const eTransIn = selectedEffect.transitionIn ?? 0;
                const eTransOut = selectedEffect.transitionOut ?? 0;
                const eReverse = selectedEffect.reverse ?? false;
                const eHold = Math.max(0, eDur - eTransIn - (eReverse ? eTransOut : 0));
                const labels: Record<string, { inLabel: string; outLabel: string; holdLabel: string }> = {
                  'zoom-pan': { inLabel: 'Zoom In', outLabel: 'Zoom Out', holdLabel: 'Hold' },
                  'spotlight': { inLabel: 'Fade In', outLabel: 'Fade Out', holdLabel: 'Hold' },
                  'blur': { inLabel: 'Blur In', outLabel: 'Blur Out', holdLabel: 'Hold' },
                  'text': { inLabel: 'Appear', outLabel: 'Disappear', holdLabel: 'Visible' },
                  'fade': { inLabel: 'Fade In', outLabel: 'Fade Out', holdLabel: 'Hold' },
                };
                const l = labels[selectedEffect.type] || labels['zoom-pan'];
                return (
                  <>
                    <div style={{ width: 8, height: 8, borderRadius: 2, background: eMeta.color, flexShrink: 0 }} />
                    <span style={{ fontSize: 11, color: eMeta.color, fontWeight: 600, flexShrink: 0 }}>{eMeta.label}</span>
                    <span style={{ fontSize: 10, color: C.muted, flexShrink: 0 }}>{eDur.toFixed(2)}s</span>
                    <div style={{ width: 1, height: 18, background: C.border, flexShrink: 0 }} />
                    <span style={{ fontSize: 9, color: C.muted, flexShrink: 0 }}>{l.inLabel}</span>
                    <NumericInput value={eTransIn} min={0} max={eDur} width={34} color={eMeta.color}
                      onChange={(v) => updateEffect(selectedEffect.id, { transitionIn: Math.min(v, eDur - (eReverse ? eTransOut : 0)) })} />
                    <span style={{ fontSize: 9, color: C.dim, fontWeight: 600, flexShrink: 0 }}>{l.holdLabel}: {eHold.toFixed(2)}s</span>
                    <label style={{ display: "flex", alignItems: "center", gap: 3, cursor: "pointer", fontSize: 9, color: eReverse ? eMeta.color : C.muted, fontWeight: 600, flexShrink: 0 }}>
                      <input type="checkbox" checked={eReverse}
                        onChange={(e) => updateEffect(selectedEffect.id, { reverse: e.target.checked, transitionOut: e.target.checked && eTransOut === 0 ? eTransIn : eTransOut })}
                        style={{ accentColor: eMeta.color, width: 11, height: 11 }} />
                      Return
                    </label>
                    <span style={{ fontSize: 9, color: C.muted, flexShrink: 0, visibility: eReverse ? "visible" : "hidden" }}>{l.outLabel}</span>
                    <NumericInput value={eTransOut} min={0} max={eDur - eTransIn} width={34} color={eMeta.color}
                      onChange={(v) => updateEffect(selectedEffect.id, { transitionOut: v })}
                      style={{ visibility: eReverse ? "visible" : "hidden" }} />
                  </>
                );
              })() : (
                <span style={{ fontSize: 11, color: C.muted }}>Click timeline to place playhead. S to split. F to freeze frame.</span>
              )}
            </div>
            {/* Right: Delete + Add Effect — always pinned */}
            <div style={{ display: "flex", alignItems: "center", gap: 6, flexShrink: 0 }}>
              {selClip && (
                <Button variant="danger" size="sm" onClick={() => { if (selectedClipIndex !== null && clips.length > 1) deleteClip(selectedClipIndex); }} disabled={clips.length <= 1}>Delete</Button>
              )}
              {selectedEffect && (
                <Button variant="danger" size="sm" onClick={() => { removeEffect(selectedEffect.id); setIsEditingZoomPan(false); }}>Delete</Button>
              )}
              <div style={{ position: "relative" }}>
                <Button variant="secondary" size="sm" onClick={() => setShowAddEffectMenu((v) => !v)}>+ Effect</Button>
                {showAddEffectMenu && (
                  <>
                  <div onClick={() => setShowAddEffectMenu(false)} style={{ position: "fixed", inset: 0, zIndex: 29 }} />
                  <div style={{ position: "absolute", bottom: 32, right: 0, zIndex: 30, background: "#1a1a24", border: `1px solid ${C.border}`, borderRadius: 6, padding: 4, minWidth: 130, boxShadow: "0 8px 24px rgba(0,0,0,0.5)" }}>
                    {(Object.entries(EFFECT_META) as [EffectType, { label: string; color: string }][]).map(([type, meta]) => (
                      <div key={type}
                        onClick={() => {
                          const start = outputTime;
                          const end = Math.min(start + 5, outputDuration);
                          if (end - start < 0.5) return;
                          const defaults: Record<string, Partial<Omit<TimelineEffect, 'id'>>> = {
                            'zoom-pan': { transitionIn: 1, transitionOut: 1, reverse: true, zoomPan: { ...DEFAULT_ZOOM_PAN } },
                            'spotlight': { transitionIn: 0.5, transitionOut: 0.5, reverse: true, spotlight: { x: 0.5, y: 0.5, radius: 0.15, dimOpacity: 0.7 } },
                            'blur': { transitionIn: 0.3, transitionOut: 0.3, reverse: true, blur: { x: 0.3, y: 0.3, width: 0.4, height: 0.4, radius: 20 } },
                            'text': { transitionIn: 0.3, transitionOut: 0.3, reverse: true, text: { content: 'Text', x: 0.5, y: 0.5, fontSize: 5, color: '#ffffff', fontFamily: 'Inter, system-ui, sans-serif', bold: true, italic: false, underline: false, background: '', align: 'center' as const } },
                            'fade': { transitionIn: 1, transitionOut: 1, reverse: true, fade: { color: '#000000', opacity: 1 } },
                          };
                          addEffect({ type, startTime: start, endTime: end, ...defaults[type] });
                          setShowAddEffectMenu(false);
                        }}
                        style={{ padding: "6px 10px", borderRadius: 4, cursor: "pointer", display: "flex", alignItems: "center", gap: 8, fontSize: 12, color: C.text, transition: "background 0.1s" }}
                        onMouseEnter={(e) => { e.currentTarget.style.background = "rgba(255,255,255,0.06)"; }}
                        onMouseLeave={(e) => { e.currentTarget.style.background = "transparent"; }}
                      >
                        <div style={{ width: 8, height: 8, borderRadius: 2, background: meta.color, flexShrink: 0 }} />
                        {meta.label}
                      </div>
                    ))}
                  </div>
                  </>
                )}
              </div>
            </div>
          </div>
          {/* ROW 2: Effect-specific controls (or clip speed) — only shows when something is selected */}
          {(selClip || selectedEffect) && (
            <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "4px 12px 6px", minHeight: 30, borderTop: `1px solid rgba(255,255,255,0.03)` }}>
              {selClip && selClip.type !== 'freeze' && (
                <>
                  <span style={{ fontSize: 10, color: C.muted }}>Speed</span>
                  <input type="range" min="0.25" max="30" step="0.25" value={selClip.speed}
                    onChange={(e) => setClipSpeedLive(selectedClipIndex!, parseFloat(e.target.value))}
                    onMouseUp={() => commitSpeedChange()}
                    onTouchEnd={() => commitSpeedChange()}
                    style={{ width: 80, accentColor: "#6366f1" }} />
                  <NumericInput value={selClip.speed} min={0.25} max={30} width={36} color={selClip.speed !== 1 ? "#a855f7" : C.dim}
                    onChange={(v) => setClipSpeed(selectedClipIndex!, v)} />
                  <span style={{ fontSize: 10, color: C.muted }}>x</span>
                  {selClip.speed > 1 && (
                    <>
                      <span style={{ fontSize: 9, color: C.muted }}>
                        ({((selClip.sourceEnd - selClip.sourceStart)).toFixed(2)}s→{((selClip.sourceEnd - selClip.sourceStart) / selClip.speed).toFixed(2)}s)
                      </span>
                      <div style={{ display: "flex", gap: 2, background: "rgba(255,255,255,0.03)", borderRadius: 5, padding: 2 }}>
                        <button onClick={() => setClipSkipFrames(selectedClipIndex!, false)} title="Fast-forward" style={{
                          padding: "2px 6px", borderRadius: 4, border: "none", fontSize: 9, fontWeight: 600,
                          background: !selClip.skipFrames ? "rgba(168,85,247,0.2)" : "transparent",
                          color: !selClip.skipFrames ? "#a855f7" : C.muted, cursor: "pointer", fontFamily: "inherit",
                        }}>Speed up</button>
                        <button onClick={() => setClipSkipFrames(selectedClipIndex!, true)} title="Time-lapse" style={{
                          padding: "2px 6px", borderRadius: 4, border: "none", fontSize: 9, fontWeight: 600,
                          background: selClip.skipFrames ? "rgba(245,158,11,0.2)" : "transparent",
                          color: selClip.skipFrames ? "#f59e0b" : C.muted, cursor: "pointer", fontFamily: "inherit",
                        }}>Time-lapse</button>
                      </div>
                    </>
                  )}
                </>
              )}
              {selectedEffect && selectedEffect.type === 'zoom-pan' && selectedEffect.zoomPan && (
                <div style={{ display: "flex", gap: 2, background: "rgba(255,255,255,0.03)", borderRadius: 5, padding: 2 }}>
                  {(['linear', 'ease-in', 'ease-out', 'ease-in-out'] as EasingPreset[]).map((preset) => (
                    <button key={preset} onClick={() => updateEffect(selectedEffect.id, { zoomPan: { ...selectedEffect.zoomPan!, easing: preset } })}
                      style={{
                        padding: "2px 6px", borderRadius: 4, border: "none", fontSize: 9, fontWeight: 600,
                        background: selectedEffect.zoomPan?.easing === preset ? "rgba(99,102,241,0.2)" : "transparent",
                        color: selectedEffect.zoomPan?.easing === preset ? "#818cf8" : C.muted,
                        cursor: "pointer", fontFamily: "inherit",
                      }}>
                      {preset === 'ease-in-out' ? 'Smooth' : preset === 'ease-in' ? 'Ease In' : preset === 'ease-out' ? 'Ease Out' : 'Linear'}
                    </button>
                  ))}
                </div>
              )}
              {selectedEffect && selectedEffect.type !== 'zoom-pan' && (
                <EffectInspector effect={selectedEffect} onUpdate={(partial) => updateEffect(selectedEffect.id, partial)} />
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
