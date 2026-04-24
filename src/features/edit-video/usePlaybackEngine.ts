/**
 * Playback engine for the Edit Video screen.
 *
 * Drives the preview canvas from a single source of truth (`outputTime`)
 * instead of listening to `<video>.timeupdate`. That inversion is what fixes
 * the multi-source bug: the old code took a source-time reported by the
 * shared `<video>` element and tried to *reverse* it back into a clip index,
 * which collides when two clips from different sources share overlapping
 * `[sourceStart, sourceEnd]` ranges.
 *
 * Design borrowed from omniclip (rAF-driven timecode, per-clip time
 * mapping) and libopenshot (Clip wraps a ReaderBase — video or image —
 * uniformly). See: research notes in the prior conversation turn.
 *
 *   - One HTMLVideoElement or HTMLImageElement per unique MediaRef in the
 *     pool. We dedupe on mediaRefId so speeding up N sections of the same
 *     file doesn't allocate N video elements.
 *   - A single rAF loop runs while mounted. Each tick:
 *       1. If playing, advance outputTime by `dt = performance.now() - last`.
 *       2. Resolve which clip is active via `resolveAtOutputTime` (per-clip
 *          mapping, no global inverse).
 *       3. Sync the active video element's currentTime / playbackRate /
 *          play-pause state, with drift correction above DRIFT_THRESHOLD.
 *       4. Draw the active element to the canvas.
 *   - When the active MediaRef changes (clip transition), the outgoing
 *     video is paused and the incoming one is seeked + played.
 */

import { useEffect, useRef, useState, type RefObject } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { useEditStore, type ClipResolution } from "../../stores/editStore";

/** Drift threshold (seconds). If the video element's currentTime falls more
 *  than this far from where our timeline says it should be, we re-seek.
 *  Below this we let the element's own clock coast — rewriting currentTime
 *  every frame stutters on many codecs. Remotion uses the same pattern
 *  under the name `acceptableTimeShiftInSeconds`. */
const DRIFT_THRESHOLD = 0.15;

export interface PlaybackEngineOptions {
  canvasRef: RefObject<HTMLCanvasElement | null>;
  /** True when the user is pressing Play. */
  isPlaying: boolean;
  /** Current output-timeline position. The engine both reads and writes this
   *  (via `setOutputTime`) while playing. */
  outputTime: number;
  setOutputTime: (t: number) => void;
  /** Called when the playhead reaches `getOutputDuration()`. The caller is
   *  expected to set `isPlaying = false`. */
  onPlaybackEnd: () => void;
  /** True while the user is actively scrubbing the playhead. The engine
   *  stops advancing time but keeps redrawing, so scrubbing stays smooth. */
  dragging: boolean;
}

export interface PlaybackEngineResult {
  /** Natural dimensions of the currently-shown source. Used by the caller
   *  to compute letterbox rects for effect overlays. Updates when the
   *  active clip switches to a differently-sized source. */
  activeSourceSize: { width: number; height: number };
  /** Full resolution for the current tick. Useful for effect overlays that
   *  need to know which clip is active (e.g. the zoom-pan preview). */
  activeResolution: ClipResolution | null;
}

type PooledElement = HTMLVideoElement | HTMLImageElement;

export function usePlaybackEngine({
  canvasRef,
  isPlaying,
  outputTime,
  setOutputTime,
  onPlaybackEnd,
  dragging,
}: PlaybackEngineOptions): PlaybackEngineResult {
  // Mirror props into refs so the rAF loop doesn't need to re-bind every
  // time they change (re-binding would cancel/restart the loop).
  const isPlayingRef = useRef(isPlaying);
  const outputTimeRef = useRef(outputTime);
  const draggingRef = useRef(dragging);
  useEffect(() => {
    isPlayingRef.current = isPlaying;
  }, [isPlaying]);
  useEffect(() => {
    outputTimeRef.current = outputTime;
  }, [outputTime]);
  useEffect(() => {
    draggingRef.current = dragging;
  }, [dragging]);

  // Keep the LATEST setOutputTime / onPlaybackEnd callbacks reachable from
  // the loop without restarting it on each render.
  const setOutputTimeRef = useRef(setOutputTime);
  const onPlaybackEndRef = useRef(onPlaybackEnd);
  useEffect(() => {
    setOutputTimeRef.current = setOutputTime;
  }, [setOutputTime]);
  useEffect(() => {
    onPlaybackEndRef.current = onPlaybackEnd;
  }, [onPlaybackEnd]);

  const poolRef = useRef<Map<string, PooledElement>>(new Map());
  /** Path we last loaded into each pool entry. Lets us reseat the element
   *  when setPrimaryMediaRef updates the primary from `''` to a real path. */
  const poolPathRef = useRef<Map<string, string>>(new Map());
  const activeRefIdRef = useRef<string | null>(null);
  const lastTickMsRef = useRef<number>(performance.now());

  const [activeSourceSize, setActiveSourceSize] = useState({ width: 0, height: 0 });
  const [activeResolution, setActiveResolution] = useState<ClipResolution | null>(null);

  // ── Element pool management ────────────────────────────────────────────
  //
  // For every MediaRef in the pool with a non-empty path, ensure there's a
  // matching hidden <video> or <Image> loaded with that source. When the
  // path changes (primary is resolved from '' to a real path after
  // setVideoFile) we tear down and recreate so the new src is loaded.
  const mediaPool = useEditStore((s) => s.mediaPool);
  useEffect(() => {
    const pool = poolRef.current;
    const pathMap = poolPathRef.current;
    const seen = new Set<string>();
    for (const [id, ref] of Object.entries(mediaPool)) {
      seen.add(id);
      const prevPath = pathMap.get(id);
      const pathChanged = prevPath !== undefined && prevPath !== ref.path;
      if (pool.has(id) && !pathChanged) continue;
      // Tear down old entry if the path changed
      if (pathChanged) {
        const old = pool.get(id);
        if (old instanceof HTMLVideoElement) {
          old.pause();
          old.removeAttribute("src");
          old.load();
        }
        pool.delete(id);
      }
      if (!ref.path) {
        // Primary placeholder before setPrimaryMediaRef has filled in a real path.
        pathMap.set(id, ref.path);
        continue;
      }
      const src = convertFileSrc(ref.path);
      if (ref.kind === "video") {
        const el = document.createElement("video");
        el.playsInline = true;
        el.preload = "auto";
        // Don't mute — users may want to hear the source while scrubbing.
        el.src = src;
        el.load();
        pool.set(id, el);
      } else {
        const el = new Image();
        el.decoding = "async";
        el.src = src;
        pool.set(id, el);
      }
      pathMap.set(id, ref.path);
    }
    // Remove entries that no longer exist in the pool
    for (const id of Array.from(pool.keys())) {
      if (seen.has(id)) continue;
      const el = pool.get(id);
      if (el instanceof HTMLVideoElement) {
        el.pause();
        el.removeAttribute("src");
        el.load();
      }
      pool.delete(id);
      pathMap.delete(id);
      if (activeRefIdRef.current === id) activeRefIdRef.current = null;
    }
  }, [mediaPool]);

  // ── rAF loop ──────────────────────────────────────────────────────────
  //
  // This effect mounts once and runs for the life of the component. All
  // props/state it needs are reached via refs so the loop isn't torn down
  // when outputTime, isPlaying, etc. change. The loop itself reads fresh
  // store state each tick via `useEditStore.getState()` — no subscription
  // churn, no stale closure.
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    let raf = 0;
    let cancelled = false;
    // Reset the delta-time baseline so the first tick doesn't see the whole
    // time elapsed since mount as `dt`.
    lastTickMsRef.current = performance.now();

    const loop = () => {
      if (cancelled) return;
      const now = performance.now();
      // Clamp dt so a long pause (tab unfocused) doesn't jump the playhead
      // minutes ahead on the next tick.
      const dt = Math.min(0.25, (now - lastTickMsRef.current) / 1000);
      lastTickMsRef.current = now;

      const state = useEditStore.getState();
      const totalDur = state.getOutputDuration();
      let cur = outputTimeRef.current;

      // Advance outputTime while playing (and not being scrubbed).
      if (isPlayingRef.current && !draggingRef.current && totalDur > 0) {
        const next = cur + dt;
        if (next >= totalDur) {
          cur = totalDur;
          outputTimeRef.current = totalDur;
          setOutputTimeRef.current(totalDur);
          onPlaybackEndRef.current();
        } else {
          cur = next;
          outputTimeRef.current = next;
          setOutputTimeRef.current(next);
        }
      }

      // Resolve which clip is active for this output time.
      const res = state.resolveAtOutputTime(cur);
      setActiveResolution((prev) => {
        // Avoid re-render when the clip index hasn't changed — the other
        // fields (localOutputTime, sourceTime) change every frame but the
        // consumers that care about those read directly from their own refs.
        if (prev && res && prev.clipIndex === res.clipIndex && prev.mediaRef?.id === res.mediaRef?.id) {
          return prev;
        }
        return res;
      });

      // Sync the active media element.
      const pool = poolRef.current;
      const prevActiveId = activeRefIdRef.current;
      const activeId = res?.mediaRef?.id ?? null;

      // Pause the outgoing element if we changed clips to a different source.
      if (prevActiveId && prevActiveId !== activeId) {
        const prevEl = pool.get(prevActiveId);
        if (prevEl instanceof HTMLVideoElement && !prevEl.paused) {
          prevEl.pause();
        }
      }

      if (res && res.mediaRef) {
        const el = pool.get(res.mediaRef.id);
        if (el instanceof HTMLVideoElement) {
          syncVideo(el, res, isPlayingRef.current, prevActiveId !== activeId);
          if (el.videoWidth > 0 && el.videoHeight > 0) {
            setActiveSourceSize((s) =>
              s.width === el.videoWidth && s.height === el.videoHeight
                ? s
                : { width: el.videoWidth, height: el.videoHeight },
            );
          }
        } else if (el instanceof HTMLImageElement) {
          if (el.naturalWidth > 0 && el.naturalHeight > 0) {
            setActiveSourceSize((s) =>
              s.width === el.naturalWidth && s.height === el.naturalHeight
                ? s
                : { width: el.naturalWidth, height: el.naturalHeight },
            );
          }
        }
      } else {
        // No active media → stop everything.
        for (const el of pool.values()) {
          if (el instanceof HTMLVideoElement && !el.paused) el.pause();
        }
      }
      activeRefIdRef.current = activeId;

      // Draw the active source to the canvas.
      drawActiveToCanvas(ctx, canvas, res, pool);

      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => {
      cancelled = true;
      cancelAnimationFrame(raf);
    };
    // canvasRef is stable; the loop reads everything else through refs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Unmount cleanup — make sure no video keeps playing after the engine dies.
  useEffect(() => {
    return () => {
      for (const el of poolRef.current.values()) {
        if (el instanceof HTMLVideoElement) {
          el.pause();
          el.removeAttribute("src");
          el.load();
        }
      }
      poolRef.current.clear();
      poolPathRef.current.clear();
    };
  }, []);

  return { activeSourceSize, activeResolution };
}

/** Bring a video element into the state dictated by the current clip.
 *  Called every tick — must be cheap in the steady state (no redundant
 *  property writes that could stutter playback). */
function syncVideo(
  el: HTMLVideoElement,
  res: ClipResolution,
  isPlaying: boolean,
  isTransition: boolean,
): void {
  const clip = res.clip;

  if (clip.type === "freeze") {
    // Freeze: pause on a single frame. Speed field is meaningless here.
    if (!el.paused) el.pause();
    if (Math.abs(el.currentTime - res.sourceTime) > 0.05) {
      el.currentTime = res.sourceTime;
    }
    return;
  }

  const wantRate = Math.max(0.01, clip.speed);
  if (Math.abs(el.playbackRate - wantRate) > 0.01) {
    el.playbackRate = wantRate;
  }

  if (isTransition) {
    // Hard-seek on clip transition — the previous clip may have left the
    // element at a completely different currentTime.
    el.currentTime = res.sourceTime;
  } else {
    const drift = Math.abs(el.currentTime - res.sourceTime);
    if (drift > DRIFT_THRESHOLD) {
      el.currentTime = res.sourceTime;
    }
  }

  if (isPlaying) {
    if (el.paused) {
      // play() is async and can reject if the element isn't ready yet or
      // if a seek is in flight — swallow those; the next tick will retry.
      el.play().catch(() => undefined);
    }
  } else if (!el.paused) {
    el.pause();
  }
}

/** Draw the current source frame to the preview canvas, preserving aspect
 *  ratio via letterbox/pillarbox. This matches the previous CSS
 *  `object-fit: contain` behavior so effect overlays positioned in video
 *  coordinates still land on the right pixels. */
function drawActiveToCanvas(
  ctx: CanvasRenderingContext2D,
  canvas: HTMLCanvasElement,
  res: ClipResolution | null,
  pool: Map<string, PooledElement>,
): void {
  ctx.fillStyle = "#000";
  ctx.fillRect(0, 0, canvas.width, canvas.height);

  if (!res || !res.mediaRef) return;
  const el = pool.get(res.mediaRef.id);
  if (!el) return;

  let srcW = 0;
  let srcH = 0;
  if (el instanceof HTMLVideoElement) {
    srcW = el.videoWidth;
    srcH = el.videoHeight;
    // readyState >= 2 means we have at least one frame we can draw.
    if (el.readyState < 2 || srcW === 0) return;
  } else {
    srcW = el.naturalWidth;
    srcH = el.naturalHeight;
    if (srcW === 0) return;
  }

  // object-fit: contain
  const cW = canvas.width;
  const cH = canvas.height;
  const srcAspect = srcW / srcH;
  const dstAspect = cW / cH;
  let drawW: number;
  let drawH: number;
  if (srcAspect > dstAspect) {
    drawW = cW;
    drawH = cW / srcAspect;
  } else {
    drawH = cH;
    drawW = cH * srcAspect;
  }
  const dx = (cW - drawW) / 2;
  const dy = (cH - drawH) / 2;
  try {
    ctx.drawImage(el, dx, dy, drawW, drawH);
  } catch {
    // drawImage can throw "HTMLImageElement is not decoded" while the image
    // is still loading; next tick will retry.
  }
}
