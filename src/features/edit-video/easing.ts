import type { EditClip, EasingPreset, TimelineEffect, ZoomPanEffect } from "../../stores/editStore";
import { clipOutputDuration } from "../../stores/editStore";

export function applyEasing(t: number, preset: EasingPreset): number {
  const clamped = Math.max(0, Math.min(1, t));
  switch (preset) {
    case 'linear': return clamped;
    case 'ease-in': return clamped * clamped;
    case 'ease-out': return clamped * (2 - clamped);
    case 'ease-in-out': return clamped < 0.5 ? 2 * clamped * clamped : -1 + (4 - 2 * clamped) * clamped;
  }
}

/**
 * Compute the animation progress (0-1) for an effect with transition phases.
 *
 * Without reverse: goes from 0→1 over transitionIn, stays at 1 for the rest.
 * With reverse:    0→1 over transitionIn, holds at 1, then 1→0 over transitionOut.
 *
 * Returns the interpolation factor to use between start and end states.
 */
export function effectProgress(
  localTime: number,
  totalDuration: number,
  transitionIn: number,
  transitionOut: number,
  reverse: boolean,
  easing: EasingPreset,
): number {
  if (totalDuration <= 0) return 0;

  if (reverse) {
    // Three phases: zoom in → hold → zoom out
    const holdStart = transitionIn;
    const holdEnd = totalDuration - transitionOut;

    if (localTime <= 0) return 0;
    if (transitionIn > 0 && localTime < holdStart) {
      // Zooming in
      return applyEasing(localTime / transitionIn, easing);
    }
    if (localTime <= Math.max(holdStart, holdEnd)) {
      // Holding at full zoom
      return 1;
    }
    if (transitionOut > 0 && localTime < totalDuration) {
      // Zooming out (reversing)
      const outProgress = (localTime - holdEnd) / transitionOut;
      return applyEasing(1 - outProgress, easing);
    }
    return 0;
  } else {
    // Simple: 0→1 over transitionIn, then hold at 1
    if (transitionIn > 0 && transitionIn < totalDuration && localTime < transitionIn) {
      return applyEasing(localTime / transitionIn, easing);
    }
    // No transition or past it — interpolate over full duration
    if (transitionIn <= 0 || transitionIn >= totalDuration) {
      return applyEasing(localTime / totalDuration, easing);
    }
    return 1;
  }
}

/**
 * Compute the CSS transform for a zoom/pan effect (legacy clip-level).
 */
export function computeZoomTransform(
  clip: EditClip,
  clipLocalTime: number,
  videoWidth: number,
  videoHeight: number,
): { scale: number; tx: number; ty: number } {
  if (!clip.zoomPan) return { scale: 1, tx: 0, ty: 0 };
  const duration = clipOutputDuration(clip);
  const t = duration > 0 ? clipLocalTime / duration : 0;
  const easedT = applyEasing(t, clip.zoomPan.easing);
  return interpolateZoom(clip.zoomPan, easedT, videoWidth, videoHeight);
}

/**
 * Interpolate between start and end zoom regions using a progress factor (0-1).
 */
function interpolateZoom(
  zoomPan: ZoomPanEffect,
  progress: number,
  videoWidth: number,
  videoHeight: number,
): { scale: number; tx: number; ty: number } {
  const { startRegion, endRegion } = zoomPan;

  // Defensive: malformed region data (NaN, Infinity) would produce a broken
  // CSS transform and blank the video preview. Fall back to identity.
  const fields = [
    startRegion.x, startRegion.y, startRegion.width, startRegion.height,
    endRegion.x, endRegion.y, endRegion.width, endRegion.height,
  ];
  if (!fields.every((v) => Number.isFinite(v))) {
    return { scale: 1, tx: 0, ty: 0 };
  }

  const x = startRegion.x + (endRegion.x - startRegion.x) * progress;
  const y = startRegion.y + (endRegion.y - startRegion.y) * progress;
  const w = startRegion.width + (endRegion.width - startRegion.width) * progress;
  const h = startRegion.height + (endRegion.height - startRegion.height) * progress;

  // Clamp region to avoid extreme zoom that makes the preview unusable
  // (min 5% = 20x zoom max). Also enforce scale >= 1 so we never shrink.
  const safeW = Math.max(w, 0.05);
  const safeH = Math.max(h, 0.05);
  const scale = Math.max(1, Math.min(1 / safeW, 1 / safeH, 20));

  // Clamp translate so the visible window always shows part of the image.
  // Valid range: tx in [-W*(scale-1), 0]. Anything outside pushes the image
  // entirely off-screen and the preview goes black.
  const rawTx = -x * videoWidth * scale;
  const rawTy = -y * videoHeight * scale;
  const tx = Math.max(-videoWidth * (scale - 1), Math.min(0, rawTx));
  const ty = Math.max(-videoHeight * (scale - 1), Math.min(0, rawTy));

  if (!Number.isFinite(scale) || !Number.isFinite(tx) || !Number.isFinite(ty)) {
    return { scale: 1, tx: 0, ty: 0 };
  }

  return { scale, tx, ty };
}

/**
 * Find the active zoom effect at a given output time and compute the transform.
 * Supports transition phases and reverse (zoom in → hold → zoom out).
 */
export function computeZoomAtTime(
  effects: TimelineEffect[],
  outputTime: number,
  videoWidth: number,
  videoHeight: number,
): { scale: number; tx: number; ty: number } {
  for (const effect of effects) {
    if (effect.type === 'zoom-pan' && effect.zoomPan && outputTime >= effect.startTime && outputTime <= effect.endTime) {
      const localTime = outputTime - effect.startTime;
      const duration = effect.endTime - effect.startTime;
      const transIn = effect.transitionIn ?? duration; // default: animate over full duration
      const transOut = effect.transitionOut ?? 0;
      const reverse = effect.reverse ?? false;

      const progress = effectProgress(localTime, duration, transIn, transOut, reverse, effect.zoomPan.easing);
      return interpolateZoom(effect.zoomPan, progress, videoWidth, videoHeight);
    }
  }
  return { scale: 1, tx: 0, ty: 0 };
}

/**
 * Compute effect opacity for non-zoom effects (spotlight, blur, text, fade).
 * With reverse: fades in → holds → fades out.
 * Without reverse: fades in → holds.
 */
export function effectOpacity(
  localTime: number,
  totalDuration: number,
  transitionIn: number,
  transitionOut: number,
  reverse: boolean,
): number {
  return effectProgress(localTime, totalDuration, transitionIn, transitionOut, reverse, 'ease-out');
}
