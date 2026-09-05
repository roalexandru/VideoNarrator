/**
 * Dead-air trimming: turn detected silence spans into a tighter clip list.
 *
 * Pure on purpose. The whole feature reduces to "subtract some source-time
 * ranges from the clips," which the existing render pipeline already handles —
 * a trimmed timeline is just more `EditClip`s with narrower source ranges, so
 * nothing downstream (edit plan, hashing, compositor) needs to learn about
 * silence at all.
 */

import type { EditClip } from "../stores/editStore";
import type { SilenceSpan } from "./tauri/commands";

export interface SilenceTrimOptions {
  /**
   * Ignore silences shorter than this (seconds). Detection runs loose so the
   * user can tune down, but cutting a 0.2 s breath reads as clipped speech
   * rather than tighter pacing.
   */
  minSilence: number;
  /**
   * Leave this much of each silence in place, at both edges. Cutting flush to
   * the detected boundary clips word onsets and sounds abrupt; a short pad
   * keeps the splice natural.
   */
  padding: number;
  /**
   * Discard surviving fragments shorter than this (seconds). Sub-frame slivers
   * are worse than the silence they replaced — ffmpeg spends a keyframe on
   * them and they read as a stutter.
   */
  minClip: number;
}

export const DEFAULT_TRIM_OPTIONS: SilenceTrimOptions = {
  minSilence: 0.8,
  padding: 0.15,
  minClip: 0.3,
};

export interface SilenceTrimResult {
  clips: EditClip[];
  /** Source seconds removed. Note this is *source* time — a clip with
   *  `speed != 1` contributes less than this to the output timeline. */
  removedSeconds: number;
  /** How many distinct ranges were cut. 0 means nothing changed. */
  cuts: number;
  /** Clips dropped entirely because they were nothing but silence. */
  clipsDropped: number;
}

interface Range {
  start: number;
  end: number;
}

/** Merge overlapping/touching ranges. Input need not be sorted. */
function mergeRanges(ranges: Range[]): Range[] {
  if (ranges.length === 0) return [];
  const sorted = [...ranges].sort((a, b) => a.start - b.start);
  const out: Range[] = [{ ...sorted[0] }];
  for (const r of sorted.slice(1)) {
    const last = out[out.length - 1];
    if (r.start <= last.end) {
      last.end = Math.max(last.end, r.end);
    } else {
      out.push({ ...r });
    }
  }
  return out;
}

/** Subtract `cuts` (assumed merged and sorted) from a single range. */
function subtractRanges(range: Range, cuts: Range[]): Range[] {
  let pieces: Range[] = [{ ...range }];
  for (const cut of cuts) {
    const next: Range[] = [];
    for (const p of pieces) {
      if (cut.end <= p.start || cut.start >= p.end) {
        next.push(p); // no overlap
        continue;
      }
      if (cut.start > p.start) next.push({ start: p.start, end: cut.start });
      if (cut.end < p.end) next.push({ start: cut.end, end: p.end });
    }
    pieces = next;
  }
  return pieces;
}

/**
 * Rebuild `clips` with silent stretches removed.
 *
 * `spans` are in **source** time for one specific media file, so only clips
 * pointing at that file are touched — a timeline mixing two recordings must
 * not have one file's silence map applied to the other's source range.
 * `freeze` and `image` clips are skipped: they have no source audio, and their
 * duration comes from `freezeDuration`/`imageDuration` rather than a range.
 *
 * Never returns an empty timeline. If every clip would be cut away (a
 * recording that is silence end to end), the original list comes back with
 * `cuts: 0` — an empty timeline has nothing to render and no way back except
 * undo, which is a worse outcome than declining the edit.
 */
export function removeSilences(
  clips: EditClip[],
  spans: SilenceSpan[],
  /** The media the spans were detected on. */
  analyzedMediaRefId: string | null | undefined,
  /** Clips with a null `mediaRefId` implicitly point at the primary media. */
  primaryMediaRefId: string | null | undefined,
  options: Partial<SilenceTrimOptions> = {},
): SilenceTrimResult {
  const opts = { ...DEFAULT_TRIM_OPTIONS, ...options };
  const minSilence = Math.max(0, opts.minSilence);
  const padding = Math.max(0, opts.padding);
  const minClip = Math.max(0, opts.minClip);

  const unchanged: SilenceTrimResult = {
    clips,
    removedSeconds: 0,
    cuts: 0,
    clipsDropped: 0,
  };

  // Normalize spans up front: drop reversed/degenerate ones, keep only those
  // long enough to be worth cutting, then shrink each by the padding.
  const candidateCuts = mergeRanges(
    spans
      .filter((s) => Number.isFinite(s.start) && Number.isFinite(s.end))
      .map((s) => ({ start: Math.min(s.start, s.end), end: Math.max(s.start, s.end) }))
      .filter((s) => s.end - s.start >= minSilence)
      .map((s) => ({ start: s.start + padding, end: s.end - padding }))
      .filter((s) => s.end > s.start),
  );

  if (candidateCuts.length === 0) return unchanged;

  const belongsToAnalyzed = (clip: EditClip): boolean => {
    const ref = clip.mediaRefId ?? primaryMediaRefId ?? null;
    const analyzed = analyzedMediaRefId ?? primaryMediaRefId ?? null;
    return ref === analyzed;
  };

  const out: EditClip[] = [];
  let removedSeconds = 0;
  let cuts = 0;
  let clipsDropped = 0;

  for (const clip of clips) {
    const kind = clip.type ?? "normal";
    if (kind !== "normal" || !belongsToAnalyzed(clip)) {
      out.push(clip);
      continue;
    }

    const range: Range = { start: clip.sourceStart, end: clip.sourceEnd };
    if (!(range.end > range.start)) {
      out.push(clip);
      continue;
    }

    // Only the cuts that actually land inside this clip.
    const localCuts = candidateCuts
      .map((c) => ({ start: Math.max(c.start, range.start), end: Math.min(c.end, range.end) }))
      .filter((c) => c.end > c.start);

    if (localCuts.length === 0) {
      out.push(clip);
      continue;
    }

    const survivors = subtractRanges(range, localCuts).filter((p) => p.end - p.start >= minClip);

    if (survivors.length === 0) {
      // The clip was silence end to end.
      clipsDropped += 1;
      removedSeconds += range.end - range.start;
      cuts += localCuts.length;
      continue;
    }

    const wasSplit = survivors.length > 1 || survivors[0].end - survivors[0].start < range.end - range.start;
    removedSeconds += range.end - range.start - survivors.reduce((a, p) => a + (p.end - p.start), 0);
    cuts += localCuts.length;

    for (const p of survivors) {
      out.push({
        ...clip,
        // A split clip needs its own identity; reusing the id would collide in
        // React keys and in the selected-clip index.
        id: wasSplit ? crypto.randomUUID() : clip.id,
        sourceStart: p.start,
        sourceEnd: p.end,
        // `zoomPan` is the legacy per-clip zoom (modern zoom lives on the
        // effects track, animated post-concat by time range). Its start/end
        // regions are defined across the clip's full span, so carrying it onto
        // every fragment would replay the whole zoom once per fragment. Keep
        // it only when the clip came through whole.
        zoomPan: wasSplit ? undefined : clip.zoomPan,
      });
    }
  }

  if (out.length === 0) return unchanged;

  return { clips: out, removedSeconds, cuts, clipsDropped };
}
