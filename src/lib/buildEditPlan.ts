/**
 * Build a `VideoEditPlan` payload for the Tauri `apply_video_edits` command
 * from the current edit store state.
 *
 * Centralizing this avoids drift between Processing (which renders edits as
 * part of the narration pipeline) and Export (which re-renders if the cache
 * is stale).
 *
 * The Rust backend expects:
 *   - `clips` with snake_case fields (EditClip struct has no rename_all)
 *   - `effects` with camelCase fields (OverlayEffect etc. use rename_all)
 *   - `zoom_pan` attached per-clip since the Rust pipeline applies zoom as a
 *     per-clip ffmpeg filter, not a post-pass overlay.
 */

import type { EditClip, TimelineEffect } from "../stores/editStore";
import type { VideoEditPlan } from "./tauri/commands";

export function buildEditPlan(
  clips: EditClip[],
  effects: TimelineEffect[],
): VideoEditPlan {
  const effectsTrack = effects || [];
  const planClips = clips.map((c) => {
    // NOTE: timeline-track zoom-pan effects are NOT mapped onto clips anymore.
    // They're animated post-concat in build_effects_filter using their own
    // time range (mirrors OpenShot's Timeline::apply_effects pattern).
    // We still honor `c.zoomPan` for backward compat with old saved projects
    // that used the legacy per-clip zoom field.
    return {
      start_seconds: c.sourceStart,
      end_seconds: c.sourceEnd,
      speed: c.speed,
      fps_override: c.fpsOverride,
      clip_type: c.type ?? "normal",
      freeze_source_time: c.freezeSourceTime,
      freeze_duration: c.freezeDuration,
      zoom_pan: c.zoomPan
        ? {
            startRegion: c.zoomPan.startRegion,
            endRegion: c.zoomPan.endRegion,
            easing: c.zoomPan.easing,
          }
        : null,
    };
  });

  // ALL timeline effects (including zoom-pan) flow through the post-concat
  // effects pass so each has its own bounded time range.
  const planEffects = effectsTrack.map((e) => ({
    type: e.type,
    startTime: e.startTime,
    endTime: e.endTime,
    transitionIn: e.transitionIn,
    transitionOut: e.transitionOut,
    reverse: e.reverse,
    spotlight: e.spotlight
      ? {
          x: e.spotlight.x,
          y: e.spotlight.y,
          radius: e.spotlight.radius,
          dimOpacity: e.spotlight.dimOpacity,
        }
      : undefined,
    blur: e.blur
      ? {
          x: e.blur.x,
          y: e.blur.y,
          width: e.blur.width,
          height: e.blur.height,
          radius: e.blur.radius,
          invert: e.blur.invert,
        }
      : undefined,
    text: e.text
      ? {
          content: e.text.content,
          x: e.text.x,
          y: e.text.y,
          fontSize: e.text.fontSize,
          color: e.text.color,
          fontFamily: e.text.fontFamily,
          bold: e.text.bold,
          italic: e.text.italic,
          underline: e.text.underline,
          background: e.text.background,
          align: e.text.align,
          opacity: e.text.opacity,
        }
      : undefined,
    fade: e.fade ? { color: e.fade.color, opacity: e.fade.opacity } : undefined,
    zoomPan: e.zoomPan
      ? {
          startRegion: e.zoomPan.startRegion,
          endRegion: e.zoomPan.endRegion,
          easing: e.zoomPan.easing,
        }
      : undefined,
  }));

  return { clips: planClips, effects: planEffects };
}

/** Heuristic: does this edit plan actually require rendering?  If there are
 *  no speed/zoom/freeze/effects the original video can be used as-is. */
export function planRequiresRender(
  clips: EditClip[],
  effects: TimelineEffect[],
): boolean {
  if (effects.some((e) => e.type !== "zoom-pan" || e.zoomPan)) return true;
  if (clips.length > 1) return true;
  const c = clips[0];
  if (!c) return false;
  if (c.speed !== 1) return true;
  if (c.fpsOverride != null) return true;
  if (c.type === "freeze") return true;
  if (c.zoomPan) return true;
  // Also triggered if the single clip doesn't cover the full source (trim).
  // Caller compares sourceStart/sourceEnd against video duration separately.
  return false;
}
