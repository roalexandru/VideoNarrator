import { useRef, useEffect, useMemo, useState, useCallback } from "react";
import { useProjectStore } from "../../stores/projectStore";
import { useEditStore, clipOutputDuration, EFFECT_META } from "../../stores/editStore";
import type { EasingPreset, ZoomPanEffect, EffectType, TimelineEffect } from "../../stores/editStore";
import { extractEditThumbnails, applyVideoEdits, openFolder } from "../../lib/tauri/commands";
import { buildEditPlan, planRequiresRender } from "../../lib/buildEditPlan";
import { computeEditPlanHash } from "../../lib/editPlanHash";
import { Channel } from "@tauri-apps/api/core";
import { save as saveDialog, message as showMessage } from "@tauri-apps/plugin-dialog";
import type { ProgressEvent } from "../../types/processing";
import { Button } from "../../components/ui/Button";
import { secondsToTimestamp } from "../../lib/formatters";
import { convertFileSrc } from "@tauri-apps/api/core";
import { trackError } from "../telemetry/analytics";
import { computeZoomTransform, computeZoomAtTime } from "./easing";
import { ZoomPanOverlay } from "./ZoomPanOverlay";
import { EffectsOverlay } from "./EffectsOverlay";
import { EffectInspector } from "./EffectInspector";
import { NumericInput } from "./NumericInput";
import { usePlaybackEngine } from "./usePlaybackEngine";

const C = { text: "#e0e0ea", dim: "#8b8ba0", muted: "#5a5a6e", border: "rgba(255,255,255,0.07)", accent: "#818cf8" };

const DEFAULT_ZOOM_PAN: ZoomPanEffect = {
  startRegion: { x: 0, y: 0, width: 1, height: 1 },
  endRegion: { x: 0.25, y: 0.25, width: 0.5, height: 0.5 },
  easing: 'ease-in-out',
};

export function EditVideoScreen() {
  const videoFile = useProjectStore((s) => s.videoFile);
  const videoAccessError = useProjectStore((s) => s.videoAccessError);
  const projectId = useProjectStore((s) => s.projectId);
  const store = useEditStore();
  const { clips, effects, selectedClipIndex, selectedEffectId, initFromVideo, splitAt, deleteClip, setClipSpeed, setClipSpeedLive, commitSpeedChange, setClipSkipFrames, moveClip, selectClip, undo, redo, canUndo, canRedo, insertFreezeFrame, setFreezeDuration, setImageDuration, setClipZoomPanLive, commitZoomPanChange, addEffect, removeEffect, updateEffect, updateEffectLive, commitEffectChange, selectEffect } = store;

  const canvasRef = useRef<HTMLCanvasElement>(null);
  const timelineRef = useRef<HTMLDivElement>(null);
  const dragHandlersRef = useRef<{ onMove: (e: MouseEvent) => void; onUp: () => void } | null>(null);
  const outputTimeRef = useRef(0); // mirrors outputTime for use in callbacks without stale closures
  const [videoDuration, setVideoDuration] = useState(0); // actual source video duration
  const [isPlaying, setIsPlaying] = useState(false);
  const [outputTime, setOutputTime] = useState(0); // position on the OUTPUT timeline
  // Thumbnails are now keyed by MediaRef.id — every unique video source gets
  // its own strip so clips from "+"-added files show their own frames.
  const [thumbsByMedia, setThumbsByMedia] = useState<Record<string, string[]>>({});
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

  // Init clips only when no edits exist — preserves edits on navigation
  useEffect(() => {
    if (videoDuration > 0 && clips.length === 0) {
      initFromVideo(videoDuration);
    }
  }, [videoDuration]);

  // Seed videoDuration from the projectStore's videoFile as soon as it's known.
  // Previously this came from <video>.loadedmetadata — we now own playback
  // directly, so we read the probed duration from projectStore instead.
  useEffect(() => {
    if (videoFile?.duration && videoFile.duration > 0) {
      setVideoDuration(videoFile.duration);
    }
  }, [videoFile?.duration]);

  // Playback engine — owns the canvas, the element pool, and the rAF loop.
  // Replaces the old <video> element + timeupdate + two custom rAF loops.
  const handlePlaybackEnd = useCallback(() => {
    setIsPlaying(false);
  }, []);
  const { activeSourceSize } = usePlaybackEngine({
    canvasRef,
    isPlaying,
    outputTime,
    setOutputTime,
    onPlaybackEnd: handlePlaybackEnd,
    dragging,
  });

  const [thumbsLoading, setThumbsLoading] = useState(false);

  // Render-edited-video action (secondary "Export video only" button).
  // This lets the user save the cut+effects video standalone, without going
  // through narration generation.
  const [renderProgress, setRenderProgress] = useState<number | null>(null);
  const handleRenderVideo = useCallback(async () => {
    if (!videoFile?.path) return;
    const defaultName = (videoFile.name || "edited").replace(/\.[^.]+$/, "") + "_edited.mp4";
    const outPath = await saveDialog({
      defaultPath: defaultName,
      filters: [{ name: "MP4 Video", extensions: ["mp4"] }],
    });
    if (!outPath) return;

    const es = useEditStore.getState();
    const plan = buildEditPlan(es.clips, es.effects, (c) => {
      const m = es.resolveClipMedia(c);
      return m ? { path: m.path, id: m.id } : null;
    }, es.primaryMediaRefId);
    const ch = new Channel<ProgressEvent>();
    ch.onmessage = (e: ProgressEvent) => {
      if (e.kind === "progress") setRenderProgress(e.percent);
    };

    setRenderProgress(0);
    try {
      const result = await applyVideoEdits(videoFile.path, outPath as string, plan, ch);
      // Also populate the cache so a subsequent Export skips the render.
      es.setEditedVideoPath(result);
      es.setEditedVideoPlanHash(computeEditPlanHash(es.clips, es.effects));
      setRenderProgress(null);
      // Offer to reveal the file — on macOS this opens Finder to the folder.
      try {
        const parent = (outPath as string).split("/").slice(0, -1).join("/");
        await openFolder(parent);
      } catch {/* non-critical */}
      await showMessage(`Rendered: ${outPath}`, { title: "Edited video saved", kind: "info" });
    } catch (err) {
      console.error("render video failed:", err);
      trackError("render_edited_video", err);
      setRenderProgress(null);
      await showMessage(String(err).replace(/^(Error: )?/, ""), { title: "Render failed", kind: "error" });
    }
  }, [videoFile?.path, videoFile?.name]);
  const canRender = !!videoFile?.path && (planRequiresRender(clips, effects) ||
    (clips[0] && Math.abs((clips[0].sourceEnd - clips[0].sourceStart) - videoDuration) > 0.5));

  // Extract one thumbnail strip per unique video MediaRef in the pool. Ran
  // once when the pool first has a real path, then re-runs incrementally as
  // "+"-added videos are registered. Already-extracted strips are kept.
  const mediaPoolForThumbs = useEditStore((s) => s.mediaPool);
  useEffect(() => {
    const videoRefs = Object.values(mediaPoolForThumbs).filter(
      (m) => m.kind === "video" && m.path && !(m.id in thumbsByMedia),
    );
    if (videoRefs.length === 0) return;
    setThumbsLoading(true);
    let cancelled = false;
    (async () => {
      for (const ref of videoRefs) {
        if (cancelled) return;
        try {
          const dur = ref.duration || 120;
          const count = dur > 600 ? 30 : dur > 300 ? 40 : 60;
          const outDir = `/tmp/narrator_edit_thumbs_${projectId || "anon"}_${ref.id}`;
          const paths = await extractEditThumbnails(ref.path, outDir, count);
          if (cancelled) return;
          setThumbsByMedia((prev) => ({
            ...prev,
            [ref.id]: paths.map((p) => convertFileSrc(p)),
          }));
        } catch (err) {
          // Non-critical — clip renders without thumbnails. Log + continue.
          console.warn(`thumbnail extract failed for ${ref.path}:`, err);
        }
      }
      if (!cancelled) setThumbsLoading(false);
    })();
    return () => {
      cancelled = true;
    };
  }, [mediaPoolForThumbs, projectId, thumbsByMedia]);

  // Match the canvas backing buffer to its displayed size so the engine's
  // drawImage letterbox math maps 1:1 to screen pixels. Uses devicePixelRatio
  // so the preview stays crisp on retina displays. Without this, the canvas
  // would default to 300x150 and drawImage would upscale blurrily.
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const resize = () => {
      const rect = canvas.getBoundingClientRect();
      const dpr = window.devicePixelRatio || 1;
      const w = Math.max(1, Math.round(rect.width * dpr));
      const h = Math.max(1, Math.round(rect.height * dpr));
      if (canvas.width !== w) canvas.width = w;
      if (canvas.height !== h) canvas.height = h;
    };
    resize();
    const observer = new ResizeObserver(resize);
    observer.observe(canvas);
    return () => observer.disconnect();
  }, []);

  // Compute the actual rendered video rect within the container
  // (object-fit:contain → letterbox/pillarbox). The effect overlay components
  // position their hit-targets in these coordinates, so we need to update
  // whenever the container resizes OR the active source's aspect changes
  // (switching to an image clip from a different video, etc.).
  useEffect(() => {
    const el = videoContainerRef.current;
    if (!el) return;
    const update = () => {
      const containerRect = el.getBoundingClientRect();
      const videoW = activeSourceSize.width || videoFile?.resolution?.width || 0;
      const videoH = activeSourceSize.height || videoFile?.resolution?.height || 0;
      if (videoW > 0 && videoH > 0 && containerRect.width > 0 && containerRect.height > 0) {
        const containerAspect = containerRect.width / containerRect.height;
        const videoAspect = videoW / videoH;
        let renderedW: number, renderedH: number;
        if (videoAspect > containerAspect) {
          renderedW = containerRect.width;
          renderedH = containerRect.width / videoAspect;
        } else {
          renderedH = containerRect.height;
          renderedW = containerRect.height * videoAspect;
        }
        setVideoContainerRect({ width: renderedW, height: renderedH, left: containerRect.left, top: containerRect.top });
      } else {
        setVideoContainerRect({ width: containerRect.width, height: containerRect.height, left: containerRect.left, top: containerRect.top });
      }
    };
    update();
    const observer = new ResizeObserver(update);
    observer.observe(el);
    return () => { observer.disconnect(); };
  }, [activeSourceSize.width, activeSourceSize.height, videoFile?.resolution?.width, videoFile?.resolution?.height]);

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

  // Seek is now a single-line operation — the playback engine reacts to
  // outputTime changes and re-syncs elements + redraws on its next tick.
  const seekToOutput = useCallback((t: number) => {
    const clamped = Math.max(0, Math.min(outputDuration, t));
    setOutputTime(clamped);
  }, [outputDuration]);

  // When the user selects a non-zoom overlay effect (spotlight / blur / text /
  // fade) while the playhead sits OUTSIDE its time window, snap the playhead
  // to the middle of the effect's window. The positioning overlay can still
  // be dragged while the effect is inactive (it shows at 0.4 opacity), but
  // the backdrop in that case is *some other* moment of the video — often
  // mid-zoom-pan. Users dragged based on what they saw and got a different
  // layout at export because the real render time was un-zoomed (or
  // differently-zoomed). Seeking inside the window means the preview
  // matches what the compositor will composite the effect onto.
  //
  // Depending on the whole `effects` array would re-fire this on every
  // drag of an effect's x/y/radius — the user would scrub out of the
  // window to fine-tune something else, touch a field, and get snapped
  // back mid-edit. Instead we only care about the selected effect's
  // type and time range; that's what changes "where this effect renders".
  const selectedTimingInfo = useMemo(() => {
    if (!selectedEffectId) return null;
    const e = effects.find((x) => x.id === selectedEffectId);
    if (!e) return null;
    return { type: e.type, startTime: e.startTime, endTime: e.endTime };
  }, [selectedEffectId, effects]);
  useEffect(() => {
    if (!selectedTimingInfo || isPlaying) return;
    if (selectedTimingInfo.type === 'zoom-pan') return;
    // Already inside window → leave the scrub position alone so the user
    // can fine-tune at their chosen frame.
    if (outputTime >= selectedTimingInfo.startTime && outputTime <= selectedTimingInfo.endTime) return;
    const mid = (selectedTimingInfo.startTime + selectedTimingInfo.endTime) / 2;
    seekToOutput(mid);
    // outputTime intentionally not in deps — we only want to re-check on
    // selection or window-timing changes, not on every scrub tick.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedTimingInfo, isPlaying, seekToOutput]);

  // Playback transitions are now handled inside usePlaybackEngine — the old
  // rAF tick that drove a single shared <video> element is gone. This is
  // where the multi-source "playhead-stuck" bug lived.

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

  // Play/pause just flips the flag — the playback engine reacts to it.
  // If we're at (or past) the end, rewind to the start before playing so
  // pressing Space on a finished clip restarts the preview.
  const togglePlay = () => {
    if (isPlaying) {
      setIsPlaying(false);
      return;
    }
    if (outputTime >= outputDuration - 0.05 && outputDuration > 0) {
      setOutputTime(0);
    }
    setIsPlaying(true);
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
      {videoAccessError && (
        <div style={{
          margin: "0 0 8px 0", padding: "10px 14px",
          background: "rgba(239,68,68,0.12)",
          border: "1px solid rgba(239,68,68,0.4)",
          borderRadius: 8, color: "#fca5a5",
          fontSize: 12, lineHeight: 1.5,
        }}>
          <div style={{ fontWeight: 700, color: "#fecaca", marginBottom: 4 }}>
            Can't read the video file — preview is blank
          </div>
          <div style={{ color: "#fca5a5" }}>
            {videoAccessError}
          </div>
          <div style={{ color: "#f87171", fontSize: 11, marginTop: 6 }}>
            Path: <code style={{ color: "#fecaca" }}>{videoFile?.path}</code>
          </div>
        </div>
      )}
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
          {/* Canvas backing buffer is sized to the container (devicePixelRatio-aware)
              by the effect below so drawImage's letterbox math works in the
              actual rendered pixel space. */}
          <canvas
            ref={canvasRef}
            style={{
              width: "100%",
              height: "100%",
              display: "block",
            }}
          />
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
            <div>videoFile.path: <span style={{ color: "#9ff" }}>{videoFile?.path?.slice(0, 80) || "(none)"}</span></div>
            <div>mediaPool size: {Object.keys(store.mediaPool).length}</div>
            <div>activeSource: <span style={{ color: activeSourceSize.width > 0 ? "#0f0" : "#f55" }}>{activeSourceSize.width} × {activeSourceSize.height}</span></div>
            <div>outputTime: {outputTime.toFixed(3)} / {outputDuration.toFixed(3)}</div>
            <div>isPlaying: {String(isPlaying)}</div>
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
        {/* Secondary: render the edited video on its own (without narration).
            Useful for users who just want the cut+effects version. */}
        <Button
          variant="secondary"
          size="sm"
          disabled={!canRender || renderProgress !== null}
          onClick={handleRenderVideo}
          title="Render the edited video (cuts + effects) without narration"
        >
          {renderProgress !== null
            ? `Rendering… ${Math.round(renderProgress)}%`
            : "Render Video"}
        </Button>
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
                  const isImage = clip.type === 'image';
                  const hasZoom = !!clip.zoomPan;
                  const clipMedia = store.resolveClipMedia(clip);
                  // Image clips use their own source file as the thumbnail;
                  // video / freeze clips read from the per-media strip.
                  const imageClipThumbSrc = isImage && clipMedia?.path ? convertFileSrc(clipMedia.path) : null;
                  // Resolve the strip for THIS clip's MediaRef. Legacy clips
                  // (mediaRefId null) fall back to the primary's strip.
                  const thumbKey = clipMedia?.id ?? store.primaryMediaRefId ?? null;
                  const mediaThumbs = thumbKey ? thumbsByMedia[thumbKey] ?? [] : [];
                  const refDuration = clipMedia?.duration ?? videoDuration ?? 0;
                  const freezeThumbIdx = isFreeze && refDuration > 0
                    ? Math.floor(((clip.freezeSourceTime ?? 0) / refDuration) * mediaThumbs.length)
                    : 0;
                  const startIdx = !isImage && refDuration > 0
                    ? (isFreeze ? freezeThumbIdx : Math.floor((clip.sourceStart / refDuration) * mediaThumbs.length))
                    : 0;
                  const endIdx = !isImage && refDuration > 0
                    ? (isFreeze ? freezeThumbIdx + 1 : Math.ceil((clip.sourceEnd / refDuration) * mediaThumbs.length))
                    : 0;
                  const clipThumbs = !isImage ? mediaThumbs.slice(startIdx, Math.max(startIdx + 1, endIdx)) : [];

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
                        border: isSel
                          ? `2px solid ${isFreeze ? "#38bdf8" : isImage ? "#10b981" : "#6366f1"}`
                          : isDragging ? "2px dashed #6366f1"
                          : isFreeze ? "1px solid rgba(56,189,248,0.3)"
                          : isImage ? "1px solid rgba(16,185,129,0.3)"
                          : "1px solid rgba(255,255,255,0.1)",
                        opacity: isDragging ? 0.5 : 1,
                        display: "flex", transition: "opacity 0.15s",
                        background: isImage ? "rgba(16,185,129,0.08)" : undefined,
                      }}>
                      {isImage && imageClipThumbSrc && (
                        <div style={{ flex: 1, backgroundImage: `url(${imageClipThumbSrc})`, backgroundSize: "cover", backgroundPosition: "center", minWidth: 1, pointerEvents: "none" }} />
                      )}
                      {!isImage && clipThumbs.map((t, j) => (
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
                      {/* Image duration badge */}
                      {isImage && (
                        <div style={{ position: "absolute", bottom: 2, left: 4, pointerEvents: "none" }}>
                          <span style={{ fontSize: 8, padding: "1px 5px", borderRadius: 3, background: "rgba(16,185,129,0.85)", color: "#fff", fontWeight: 700 }}>
                            {(clip.imageDuration ?? 3).toFixed(2)}s
                          </span>
                        </div>
                      )}
                      {/* Clip number + kind icon */}
                      <div style={{ position: "absolute", top: 2, left: 4, fontSize: 9, color: "rgba(255,255,255,0.75)", fontWeight: 600, pointerEvents: "none", textShadow: "0 1px 2px rgba(0,0,0,0.5)", display: "flex", alignItems: "center", gap: 3 }}>
                        {isFreeze && (
                          <svg width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round">
                            <rect x="6" y="4" width="4" height="16"/><rect x="14" y="4" width="4" height="16"/>
                          </svg>
                        )}
                        {isImage && (
                          <svg width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                            <rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="8.5" cy="8.5" r="1.5"/><polyline points="21 15 16 10 5 21"/>
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
                    title="Add video or image clip"
                    onClick={async (e) => {
                      e.stopPropagation();
                      const { open } = await import("@tauri-apps/plugin-dialog");
                      const file = await open({ multiple: false, filters: [{ name: "Media", extensions: ["mp4", "mov", "avi", "mkv", "webm", "m4v", "jpg", "jpeg", "png", "gif", "webp", "bmp"] }] });
                      if (!file) return;
                      try {
                        const { importAndAddMedia } = await import("./addMedia");
                        await importAndAddMedia(file as string);
                      } catch (err) {
                        console.error("Failed to add media:", err);
                        trackError("edit_add_media", err);
                        await showMessage(String(err).replace(/^(Error: )?/, ""), { title: "Couldn't add clip", kind: "error" });
                      }
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
                  <span style={{ fontSize: 11, color: selClip.type === 'freeze' ? "#38bdf8" : selClip.type === 'image' ? "#10b981" : C.dim, fontWeight: 600, flexShrink: 0 }}>
                    {selClip.type === 'freeze' ? 'Freeze' : selClip.type === 'image' ? 'Image' : 'Clip'} {(selectedClipIndex ?? 0) + 1}/{clips.length}
                  </span>
                  {selClip.type === 'freeze' ? (
                    <>
                      <span style={{ fontSize: 10, color: C.muted, flexShrink: 0 }}>at {secondsToTimestamp(selClip.freezeSourceTime ?? 0)}</span>
                      <span style={{ fontSize: 10, color: C.muted, flexShrink: 0 }}>Duration</span>
                      <NumericInput value={selClip.freezeDuration ?? 3} min={0.1} max={30} width={44} color="#38bdf8"
                        onChange={(v) => setFreezeDuration(selectedClipIndex!, v)} />
                      <span style={{ fontSize: 10, color: C.muted }}>s</span>
                    </>
                  ) : selClip.type === 'image' ? (
                    <>
                      <span style={{ fontSize: 10, color: C.muted, flexShrink: 0 }}>
                        {store.resolveClipMedia(selClip)?.path.split('/').pop() ?? 'image'}
                      </span>
                      <span style={{ fontSize: 10, color: C.muted, flexShrink: 0 }}>Duration</span>
                      <NumericInput value={selClip.imageDuration ?? 3} min={0.1} max={60} width={44} color="#10b981"
                        onChange={(v) => setImageDuration(selectedClipIndex!, v)} />
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
