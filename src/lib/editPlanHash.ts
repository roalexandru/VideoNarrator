/**
 * Compute a deterministic fingerprint for the current edit plan (clips + effects).
 * Export uses this to decide whether the cached `edited_video_path` is still
 * valid or whether the video must be re-rendered.
 *
 * The hash is stable under object-key ordering because we build a canonical
 * string representation ourselves instead of relying on JSON.stringify.
 */

import type { EditClip, TimelineEffect } from "../stores/editStore";

function hash32(s: string): string {
  // FNV-1a 32-bit — fast, non-cryptographic, deterministic. Good enough for
  // detecting "did any field change"; not for security.
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  // Convert to unsigned 32-bit hex
  return (h >>> 0).toString(16).padStart(8, "0");
}

function canonicalizeClip(c: EditClip): string {
  // Only fields that actually affect the rendered output. Order and rounding
  // must be stable; 4 decimals is more than enough since the UI rounds at
  // centisecond precision.
  const parts: string[] = [
    `s=${c.sourceStart.toFixed(4)}`,
    `e=${c.sourceEnd.toFixed(4)}`,
    `v=${c.speed.toFixed(3)}`,
    `sk=${c.skipFrames ? 1 : 0}`,
    `fps=${c.fpsOverride ?? "_"}`,
    `t=${c.type ?? "normal"}`,
    `fst=${c.freezeSourceTime ?? "_"}`,
    `fd=${c.freezeDuration ?? "_"}`,
  ];
  if (c.zoomPan) {
    const zp = c.zoomPan;
    parts.push(
      `zp=${zp.startRegion.x.toFixed(4)},${zp.startRegion.y.toFixed(4)},${zp.startRegion.width.toFixed(4)},${zp.startRegion.height.toFixed(4)}>${zp.endRegion.x.toFixed(4)},${zp.endRegion.y.toFixed(4)},${zp.endRegion.width.toFixed(4)},${zp.endRegion.height.toFixed(4)}:${zp.easing}`,
    );
  }
  return parts.join("|");
}

function canonicalizeEffect(e: TimelineEffect): string {
  const base = [
    `type=${e.type}`,
    `st=${e.startTime.toFixed(4)}`,
    `et=${e.endTime.toFixed(4)}`,
    `ti=${e.transitionIn ?? "_"}`,
    `to=${e.transitionOut ?? "_"}`,
    `rev=${e.reverse ? 1 : 0}`,
  ];
  if (e.spotlight) {
    const s = e.spotlight;
    base.push(`sp=${s.x.toFixed(4)},${s.y.toFixed(4)},${s.radius.toFixed(4)},${s.dimOpacity.toFixed(3)}`);
  }
  if (e.blur) {
    const b = e.blur;
    base.push(`bl=${b.x.toFixed(4)},${b.y.toFixed(4)},${b.width.toFixed(4)},${b.height.toFixed(4)},${b.radius.toFixed(2)},${b.invert ? 1 : 0}`);
  }
  if (e.text) {
    const t = e.text;
    // Include the text content — changing it means a new render
    base.push(`tx=${t.content}|${t.x.toFixed(4)},${t.y.toFixed(4)}|${t.fontSize.toFixed(2)}|${t.color}|${t.fontFamily ?? "_"}|${t.bold ? 1 : 0}|${t.italic ? 1 : 0}|${t.underline ? 1 : 0}|${t.background ?? "_"}|${t.align ?? "_"}|${(t.opacity ?? 1).toFixed(3)}`);
  }
  if (e.zoomPan) {
    const zp = e.zoomPan;
    base.push(
      `zp=${zp.startRegion.x.toFixed(4)},${zp.startRegion.y.toFixed(4)},${zp.startRegion.width.toFixed(4)},${zp.startRegion.height.toFixed(4)}>${zp.endRegion.x.toFixed(4)},${zp.endRegion.y.toFixed(4)},${zp.endRegion.width.toFixed(4)},${zp.endRegion.height.toFixed(4)}:${zp.easing}`,
    );
  }
  if (e.fade) {
    base.push(`fd=${e.fade.color},${e.fade.opacity.toFixed(3)}`);
  }
  return base.join("|");
}

/**
 * Compute a stable hash over the entire edit plan. Two plans that produce
 * the same rendered output will have the same hash; any meaningful change
 * produces a different hash.
 *
 * Effect order matters: the Rust compositor now applies zoom-pan first and
 * then iterates remaining overlays in array order, and `buildEditPlan` sends
 * the effects in the store's array order. So *reordering* effects in the
 * timeline can change the rendered output (e.g. fade-over-text vs
 * text-over-fade) and must invalidate the cache. We hash effects in their
 * original order. (An earlier version sorted them by `startTime` here,
 * which silently collided two plans that rendered differently.)
 */
export function computeEditPlanHash(
  clips: EditClip[],
  effects: TimelineEffect[],
): string {
  // Canonicalize clips in order (clip order = timeline sequence).
  const clipPart = clips.map(canonicalizeClip).join(";");
  // Canonicalize effects in array order (matches what buildEditPlan sends
  // and what the compositor iterates).
  const effectPart = effects.map(canonicalizeEffect).join(";");
  return hash32(`C:${clipPart}||E:${effectPart}`);
}
