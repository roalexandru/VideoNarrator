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
import { fileExists } from "./tauri/commands";
import { computeEditPlanHash } from "./editPlanHash";

export function buildEditPlan(
  clips: EditClip[],
  effects: TimelineEffect[],
  /** Resolve a clip's source file path from its mediaRefId. Callers pass
   *  `useEditStore.getState().resolveClipMedia`. When omitted, we emit no
   *  `input_path` field, matching the legacy single-source behavior and
   *  letting the Rust side fall back to the default `input_path` arg. */
  resolveClipMedia?: (clip: EditClip) => { path: string; id: string } | null,
  primaryMediaRefId?: string | null,
): VideoEditPlan {
  const effectsTrack = effects || [];
  const planClips = clips.map((c) => {
    // NOTE: timeline-track zoom-pan effects are NOT mapped onto clips anymore.
    // They're animated post-concat in build_effects_filter using their own
    // time range (mirrors OpenShot's Timeline::apply_effects pattern).
    // We still honor `c.zoomPan` for backward compat with old saved projects
    // that used the legacy per-clip zoom field.
    const media = resolveClipMedia ? resolveClipMedia(c) : null;
    // Only send `input_path` when the clip points at a NON-primary source —
    // primary clips stay `null` so the Rust `input_path` fallback path is
    // exercised (and old cached edits keep hashing the same).
    const isPrimary =
      !c.mediaRefId ||
      c.mediaRefId === primaryMediaRefId ||
      (media && media.id === primaryMediaRefId);
    return {
      start_seconds: c.sourceStart,
      end_seconds: c.sourceEnd,
      speed: c.speed,
      // "Time-lapse" mode: the Rust compositor silences a sped-up clip's audio
      // (compositor/audio.rs) instead of atempo-compressing it into chipmunk
      // playback. Video is unaffected — speed already time-compresses it. This
      // was never transmitted before, so the toggle silently did nothing.
      skip_frames: !!c.skipFrames,
      fps_override: c.fpsOverride,
      clip_type: c.type ?? "normal",
      freeze_source_time: c.freezeSourceTime,
      freeze_duration: c.freezeDuration,
      image_duration: c.imageDuration,
      input_path: !isPrimary && media ? media.path : undefined,
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
    // Fill the zoom-pan transitionIn default explicitly. The preview defaults
    // an unset transitionIn to the full window (easing.ts computeZoomAtTime),
    // but the Rust side defaults it to 0.0 — which only diverges for
    // reverse:true zoom-pans with no transitions. Sending the value keeps both
    // sides identical. (Overlay effects agree on `?? 0` already, so leave them.)
    transitionIn:
      e.type === "zoom-pan" ? (e.transitionIn ?? e.endTime - e.startTime) : e.transitionIn,
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

/** Tolerance for the trim check, mirroring the Rust fast-path `covers_full`
 *  comparison in video_edit.rs. Trims smaller than this render identically to
 *  the untrimmed source, so they don't force a render. */
const TRIM_EPSILON_S = 0.5;

/** Core heuristic: do the clip/effect settings (ignoring trim) require a
 *  render? Private — callers must go through `editsRequireRender`, which also
 *  folds in the trim check that this function deliberately omits. */
function planRequiresRender(
  clips: EditClip[],
  effects: TimelineEffect[],
): boolean {
  // ANY effect requires a render. (Previously this only counted zoom-pan,
  // which meant blur/text/spotlight/fade were skipped before frame extraction
  // — so a blur placed over sensitive content never reached the frames sent
  // to the LLM, and the AI could read what the user had redacted.)
  if (effects.length > 0) return true;
  if (clips.length > 1) return true;
  const c = clips[0];
  if (!c) return false;
  if (c.speed !== 1) return true;
  if (c.skipFrames) return true;
  if (c.fpsOverride != null) return true;
  if (c.type === "freeze") return true;
  if (c.zoomPan) return true;
  return false;
}

/**
 * SINGLE source of truth for "do the current edits require a render?".
 * Folds in the trim check that `planRequiresRender` omits — previously every
 * caller re-implemented that comparison inline (and ExportScreen forgot to,
 * so trim-only edits silently exported the untrimmed original).
 *
 * `sourceDuration` is the primary source's duration in seconds. Pass 0 when it
 * is unknown: the trim check is then skipped (fail-open to "no render"), which
 * matches the previous behavior for that case.
 */
export function editsRequireRender(
  clips: EditClip[],
  effects: TimelineEffect[],
  sourceDuration: number,
): boolean {
  if (planRequiresRender(clips, effects)) return true;
  const c = clips[0];
  if (!c || sourceDuration <= 0) return false;
  return (
    c.sourceStart > TRIM_EPSILON_S ||
    Math.abs(c.sourceEnd - sourceDuration) > TRIM_EPSILON_S
  );
}

export type EditedVideoResolution =
  | { kind: "original"; path: string }
  | { kind: "rendered"; path: string }
  | { kind: "render-required" };

/**
 * Decide which video file the export / preview should consume, given the
 * current edit state and the cached render metadata.
 *
 * - `original`: the edits don't require a render — use the untrimmed source.
 *   (Deliberately ignores any lingering `editedVideoPath`: deleting the last
 *   effect after a render must NOT keep exporting the stale rendered file.)
 * - `rendered`: a cached render exists, its plan hash matches the current
 *   plan, and the file is present on disk — reuse it, no re-render.
 * - `render-required`: edits exist but the cache is missing or stale.
 *
 * `fileExists` is injectable for tests; it defaults to the Tauri wrapper.
 */
export async function resolveEditedVideo(args: {
  clips: EditClip[];
  effects: TimelineEffect[];
  sourceDuration: number;
  originalVideoPath: string;
  editedVideoPath: string | null;
  editedVideoPlanHash: string | null;
  fileExists?: (path: string) => Promise<boolean>;
}): Promise<EditedVideoResolution> {
  const {
    clips,
    effects,
    sourceDuration,
    originalVideoPath,
    editedVideoPath,
    editedVideoPlanHash,
  } = args;

  if (!editsRequireRender(clips, effects, sourceDuration)) {
    return { kind: "original", path: originalVideoPath };
  }

  const exists = args.fileExists ?? fileExists;
  const current = computeEditPlanHash(clips, effects);
  if (
    editedVideoPath &&
    editedVideoPlanHash === current &&
    (await exists(editedVideoPath))
  ) {
    return { kind: "rendered", path: editedVideoPath };
  }
  return { kind: "render-required" };
}
