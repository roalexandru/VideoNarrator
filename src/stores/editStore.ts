import { create } from "zustand";

// ── Zoom/Pan Types ──

export interface ZoomRegion {
  x: number;      // left edge, 0-1 normalized
  y: number;      // top edge, 0-1 normalized
  width: number;  // 0-1 normalized
  height: number; // 0-1 normalized
}

export type EasingPreset = 'linear' | 'ease-in' | 'ease-out' | 'ease-in-out';

export interface ZoomPanEffect {
  startRegion: ZoomRegion;
  endRegion: ZoomRegion;
  easing: EasingPreset;
}

// ── Timeline Effect Types ──

export type EffectType = 'zoom-pan' | 'spotlight' | 'blur' | 'text' | 'fade';

export interface TimelineEffect {
  id: string;
  type: EffectType;
  startTime: number;  // output timeline seconds
  endTime: number;    // output timeline seconds
  transitionIn?: number;   // seconds for the "in" transition
  transitionOut?: number;  // seconds for the "out" transition (return to start)
  reverse?: boolean;       // when true, animates back to start state at the end
  // Effect-specific data
  zoomPan?: ZoomPanEffect;
  spotlight?: { x: number; y: number; radius: number; dimOpacity: number };
  blur?: { x: number; y: number; width: number; height: number; radius: number; invert?: boolean };
  text?: { content: string; x: number; y: number; fontSize: number; color: string; fontFamily?: string; bold?: boolean; italic?: boolean; underline?: boolean; background?: string; align?: 'left' | 'center' | 'right'; opacity?: number };
  fade?: { color: string; opacity: number };
}

// Colors and labels for each effect type
export const EFFECT_META: Record<EffectType, { label: string; color: string; icon: string }> = {
  'zoom-pan': { label: 'Zoom', color: '#6366f1', icon: 'zoom' },
  'spotlight': { label: 'Spotlight', color: '#f59e0b', icon: 'spotlight' },
  'blur': { label: 'Blur', color: '#8b5cf6', icon: 'blur' },
  'text': { label: 'Text', color: '#10b981', icon: 'text' },
  'fade': { label: 'Fade', color: '#64748b', icon: 'fade' },
};

// ── Media Pool (OpenTimelineIO-style media_reference) ──
//
// Every clip points at a MediaRef via `mediaRefId`. The pool holds one entry
// per imported source file; multiple clips can reuse the same MediaRef (e.g.
// speeding up sections of the same video). Keyed by content hash — lets us
// dedupe re-imports and survive file renames when we add relink later.
// See libopenshot's Clip(ReaderBase*) and omniclip's file_hash pattern.

export type MediaKind = 'video' | 'image';

export interface MediaRef {
  id: string;                    // stable id used by EditClip.mediaRefId
  hash: string;                  // content hash; equals id for the synthesized primary
  kind: MediaKind;
  path: string;                  // filesystem path — may be "" for the legacy primary until setPrimaryMediaRef resolves it
  duration: number;              // natural duration (videos); for images we still store a sensible default for ffmpeg
  width: number;
  height: number;
  fps?: number;                  // video only
}

/** Stable id for the project's primary (original) video. Legacy projects
 *  don't carry a MediaRef on every clip, so we treat absent mediaRefId as
 *  pointing at this synthetic entry, which is populated as soon as we know
 *  the primary video's real path/dimensions. */
export const PRIMARY_MEDIA_REF_ID = 'primary';

// ── Clip Types ──

export interface EditClip {
  id: string;
  /** Points into EditStore.mediaPool. Nullable for backward compat with
   *  projects saved before the multi-source rewrite — resolved to the
   *  primary MediaRef at read time. */
  mediaRefId?: string | null;
  sourceStart: number;
  sourceEnd: number;
  speed: number;
  skipFrames: boolean;
  fpsOverride: number | null;
  type?: 'normal' | 'freeze' | 'image';
  freezeSourceTime?: number;
  freezeDuration?: number;
  /** Image clips: how long the still shows on the output timeline. */
  imageDuration?: number;
  zoomPan?: ZoomPanEffect | null;  // kept for backward compat with old saved projects
}

/** Per-clip resolution result. The preview engine uses this to know which
 *  media element to show and at what time, independently per clip — so two
 *  clips with overlapping source ranges (e.g. the same file used twice)
 *  never collide in the reverse mapping (the bug the old onTime handler had). */
export interface ClipResolution {
  clipIndex: number;
  clip: EditClip;
  mediaRef: MediaRef | null;
  /** Seconds from the start of this clip's output window. 0 at the clip's left edge. */
  localOutputTime: number;
  /** The timestamp inside the source file to seek to. For freeze clips it's
   *  the frozen moment; for image clips it's 0 (no time coordinate). */
  sourceTime: number;
  /** Absolute output-timeline seconds where this clip begins. */
  clipOutputStart: number;
  /** Output-timeline duration of this clip. */
  clipOutputDuration: number;
}

// Snapshot for undo/redo — only the data that changes
interface ClipSnapshot {
  clips: EditClip[];
  effects: TimelineEffect[];
  selectedClipIndex: number | null;
  selectedEffectId: string | null;
}

const MAX_UNDO = 30;

/** Get the output duration of a single clip */
export function clipOutputDuration(c: EditClip): number {
  if (c.type === 'freeze') return c.freezeDuration ?? 3.0;
  if (c.type === 'image') return c.imageDuration ?? 3.0;
  return (c.sourceEnd - c.sourceStart) / c.speed;
}

interface EditStore {
  /** Pool of source media keyed by MediaRef.id. Every clip resolves to one
   *  of these via its mediaRefId (nullable → primary). */
  mediaPool: Record<string, MediaRef>;
  primaryMediaRefId: string | null;
  clips: EditClip[];
  effects: TimelineEffect[];
  selectedClipIndex: number | null;
  selectedEffectId: string | null;
  editedVideoPath: string | null;
  /** Hash of the edit plan (clips + effects) as it was when editedVideoPath
   *  was produced. Used by Export to detect a stale cache and regenerate. */
  editedVideoPlanHash: string | null;
  sourceDuration: number;

  // Undo/Redo
  undoStack: ClipSnapshot[];
  redoStack: ClipSnapshot[];
  _preSpeedSnapshot: ClipSnapshot | null;
  _preZoomPanSnapshot: ClipSnapshot | null;
  _preEffectSnapshot: ClipSnapshot | null;
  canUndo: () => boolean;
  canRedo: () => boolean;
  undo: () => void;
  redo: () => void;

  initFromVideo: (duration: number) => void;
  splitAt: (outputTime: number) => void;
  deleteClip: (index: number) => void;
  setClipSpeed: (index: number, speed: number) => void;
  setClipSpeedLive: (index: number, speed: number) => void;
  commitSpeedChange: () => void;
  setClipSkipFrames: (index: number, skip: boolean) => void;
  setClipFps: (index: number, fps: number | null) => void;
  moveClip: (fromIndex: number, toIndex: number) => void;
  /** Append a video clip. `mediaRefId` must already be registered in the pool.
   *  For legacy-style calls that pass a raw path, register via registerMedia first. */
  addClip: (mediaRefId: string, sourceStart: number, sourceEnd: number) => void;
  /** Append a still-image clip. `mediaRefId` must already be registered and of kind 'image'. */
  addImageClip: (mediaRefId: string, duration?: number) => void;
  setImageDuration: (index: number, duration: number) => void;
  selectClip: (index: number | null) => void;
  setEditedVideoPath: (path: string | null) => void;
  setEditedVideoPlanHash: (hash: string | null) => void;
  getOutputDuration: () => number;
  getClipOutputStart: (index: number) => number;
  /** Legacy accessor — returns the source time only. Prefer resolveAtOutputTime. */
  outputTimeToSource: (outputTime: number) => number;
  /** Per-clip resolver for the preview engine. Returns which clip is active
   *  at `outputTime`, its MediaRef, and the in-source time to seek to. */
  resolveAtOutputTime: (outputTime: number) => ClipResolution | null;
  reset: () => void;

  // ── Media Pool ──
  /** Register a MediaRef (or update the existing entry by id). Returns the id. */
  registerMedia: (ref: MediaRef) => string;
  /** Convenience: upsert-and-set the primary MediaRef (called from the setVideoFile flow). */
  setPrimaryMediaRef: (ref: MediaRef) => void;
  getMediaRef: (id: string | null | undefined) => MediaRef | null;
  /** Resolve the MediaRef for a clip, honouring legacy `mediaRefId === null` → primary. */
  resolveClipMedia: (clip: EditClip) => MediaRef | null;

  // Freeze frame
  insertFreezeFrame: (outputTime: number, duration?: number) => void;
  setFreezeDuration: (index: number, duration: number) => void;

  // Zoom/Pan (legacy per-clip — kept for backward compat)
  setClipZoomPan: (index: number, zoomPan: ZoomPanEffect | null) => void;
  setClipZoomPanLive: (index: number, zoomPan: ZoomPanEffect) => void;
  commitZoomPanChange: () => void;
  setClipEasing: (index: number, easing: EasingPreset) => void;

  // Effects track
  addEffect: (effect: Omit<TimelineEffect, 'id'>) => void;
  removeEffect: (id: string) => void;
  updateEffect: (id: string, partial: Partial<TimelineEffect>) => void;
  updateEffectLive: (id: string, partial: Partial<TimelineEffect>) => void;
  commitEffectChange: () => void;
  selectEffect: (id: string | null) => void;
  getEffectsAtTime: (outputTime: number) => TimelineEffect[];
}

// Helper: push current state to undo stack before mutation
function pushUndo(state: EditStore): Partial<EditStore> {
  const snapshot: ClipSnapshot = {
    clips: structuredClone(state.clips),
    effects: structuredClone(state.effects),
    selectedClipIndex: state.selectedClipIndex,
    selectedEffectId: state.selectedEffectId,
  };
  const stack = [...state.undoStack, snapshot];
  if (stack.length > MAX_UNDO) stack.shift();
  return { undoStack: stack, redoStack: [] };
}

function snapshotState(state: EditStore): ClipSnapshot {
  return {
    clips: structuredClone(state.clips),
    effects: structuredClone(state.effects),
    selectedClipIndex: state.selectedClipIndex,
    selectedEffectId: state.selectedEffectId,
  };
}

export const useEditStore = create<EditStore>((set, get) => ({
  mediaPool: {},
  primaryMediaRefId: null,
  clips: [],
  effects: [],
  selectedClipIndex: null,
  selectedEffectId: null,
  editedVideoPath: null,
  editedVideoPlanHash: null,
  sourceDuration: 0,
  undoStack: [],
  redoStack: [],
  _preSpeedSnapshot: null,
  _preZoomPanSnapshot: null,
  _preEffectSnapshot: null,

  // ── Media Pool ──

  registerMedia: (ref) => {
    set((state) => ({ mediaPool: { ...state.mediaPool, [ref.id]: ref } }));
    return ref.id;
  },

  setPrimaryMediaRef: (ref) =>
    set((state) => {
      // Always normalise the primary to PRIMARY_MEDIA_REF_ID so legacy clips
      // (mediaRefId=null) resolve correctly without a follow-up migration.
      const primary: MediaRef = { ...ref, id: PRIMARY_MEDIA_REF_ID };
      return {
        mediaPool: { ...state.mediaPool, [PRIMARY_MEDIA_REF_ID]: primary },
        primaryMediaRefId: PRIMARY_MEDIA_REF_ID,
      };
    }),

  getMediaRef: (id) => {
    if (!id) return null;
    return get().mediaPool[id] ?? null;
  },

  resolveClipMedia: (clip) => {
    const state = get();
    const id = clip.mediaRefId ?? state.primaryMediaRefId;
    return id ? state.mediaPool[id] ?? null : null;
  },

  canUndo: () => get().undoStack.length > 0,
  canRedo: () => get().redoStack.length > 0,

  undo: () =>
    set((state) => {
      if (state.undoStack.length === 0) return state;
      const stack = [...state.undoStack];
      const prev = stack.pop()!;
      const redoSnapshot = snapshotState(state);
      return { ...prev, undoStack: stack, redoStack: [...state.redoStack, redoSnapshot] };
    }),

  redo: () =>
    set((state) => {
      if (state.redoStack.length === 0) return state;
      const stack = [...state.redoStack];
      const next = stack.pop()!;
      const undoSnapshot = snapshotState(state);
      return { ...next, redoStack: stack, undoStack: [...state.undoStack, undoSnapshot] };
    }),

  initFromVideo: (duration) =>
    set((state) => {
      // Preserve any primary MediaRef that was registered via setPrimaryMediaRef
      // (called from setVideoFile) — we only synthesise a placeholder if nothing
      // is there yet, so tests calling `initFromVideo(120)` still work without
      // a real file.
      const existingPrimary = state.mediaPool[PRIMARY_MEDIA_REF_ID];
      const primary: MediaRef = existingPrimary
        ? { ...existingPrimary, duration }
        : {
            id: PRIMARY_MEDIA_REF_ID,
            hash: PRIMARY_MEDIA_REF_ID,
            kind: 'video',
            path: '',
            duration,
            width: 0,
            height: 0,
          };
      return {
        mediaPool: { ...state.mediaPool, [PRIMARY_MEDIA_REF_ID]: primary },
        primaryMediaRefId: PRIMARY_MEDIA_REF_ID,
        clips: [{
          id: crypto.randomUUID(),
          mediaRefId: PRIMARY_MEDIA_REF_ID,
          sourceStart: 0,
          sourceEnd: duration,
          speed: 1.0,
          skipFrames: false,
          fpsOverride: null,
        }],
        effects: [],
        selectedClipIndex: 0,
        selectedEffectId: null,
        editedVideoPath: null,
        editedVideoPlanHash: null,
        sourceDuration: duration,
        undoStack: [],
        redoStack: [],
      };
    }),

  splitAt: (outputTime) =>
    set((state) => {
      let cumulative = 0;
      for (let i = 0; i < state.clips.length; i++) {
        const clip = state.clips[i];
        const dur = clipOutputDuration(clip);
        // Skip freeze clips — can't split a still image
        if (clip.type === 'freeze') {
          cumulative += dur;
          continue;
        }
        if (outputTime > cumulative && outputTime < cumulative + dur) {
          const undo = pushUndo(state);
          const relativeTime = (outputTime - cumulative) * clip.speed;
          const sourceMiddle = clip.sourceStart + relativeTime;
          const left: EditClip = { ...clip, id: crypto.randomUUID(), sourceEnd: sourceMiddle, zoomPan: clip.zoomPan ? structuredClone(clip.zoomPan) : undefined };
          const right: EditClip = { ...clip, id: crypto.randomUUID(), sourceStart: sourceMiddle, zoomPan: clip.zoomPan ? structuredClone(clip.zoomPan) : undefined };
          const clips = [...state.clips];
          clips.splice(i, 1, left, right);
          return { ...undo, clips, selectedClipIndex: i };
        }
        cumulative += dur;
      }
      return state;
    }),

  deleteClip: (index) =>
    set((state) => {
      if (state.clips.length <= 1) return state;
      const undo = pushUndo(state);
      const clips = state.clips.filter((_, i) => i !== index);
      return { ...undo, clips, selectedClipIndex: clips.length > 0 ? Math.min(index, clips.length - 1) : null };
    }),

  setClipSpeed: (index, speed) =>
    set((state) => {
      const clip = state.clips[index];
      if (clip?.type === 'freeze') return state;
      const undo = pushUndo(state);
      const clips = [...state.clips];
      clips[index] = { ...clips[index], speed };
      return { ...undo, clips };
    }),

  setClipSpeedLive: (index, speed) =>
    set((state) => {
      const clip = state.clips[index];
      if (clip?.type === 'freeze') return state;
      const snapshot = state._preSpeedSnapshot || snapshotState(state);
      const clips = [...state.clips];
      clips[index] = { ...clips[index], speed };
      return { clips, _preSpeedSnapshot: snapshot };
    }),

  commitSpeedChange: () =>
    set((state) => {
      if (!state._preSpeedSnapshot) return state;
      const stack = [...state.undoStack, state._preSpeedSnapshot];
      if (stack.length > MAX_UNDO) stack.shift();
      return { undoStack: stack, redoStack: [], _preSpeedSnapshot: null };
    }),

  setClipSkipFrames: (index, skip) =>
    set((state) => {
      const undo = pushUndo(state);
      const clips = [...state.clips];
      clips[index] = { ...clips[index], skipFrames: skip };
      return { ...undo, clips };
    }),

  setClipFps: (index, fps) =>
    set((state) => {
      const undo = pushUndo(state);
      const clips = [...state.clips];
      clips[index] = { ...clips[index], fpsOverride: fps };
      return { ...undo, clips };
    }),

  moveClip: (fromIndex, toIndex) =>
    set((state) => {
      if (fromIndex === toIndex) return state;
      const undo = pushUndo(state);
      const clips = [...state.clips];
      const [moved] = clips.splice(fromIndex, 1);
      clips.splice(toIndex, 0, moved);
      return { ...undo, clips, selectedClipIndex: toIndex };
    }),

  addClip: (mediaRefId, sourceStart, sourceEnd) =>
    set((state) => {
      const undo = pushUndo(state);
      return {
        ...undo,
        clips: [
          ...state.clips,
          {
            id: crypto.randomUUID(),
            mediaRefId,
            sourceStart,
            sourceEnd,
            speed: 1.0,
            skipFrames: false,
            fpsOverride: null,
          },
        ],
        selectedClipIndex: state.clips.length,
      };
    }),

  addImageClip: (mediaRefId, duration = 3.0) =>
    set((state) => {
      const undo = pushUndo(state);
      return {
        ...undo,
        clips: [
          ...state.clips,
          {
            id: crypto.randomUUID(),
            mediaRefId,
            sourceStart: 0,
            sourceEnd: 0,
            speed: 1.0,
            skipFrames: false,
            fpsOverride: null,
            type: 'image',
            imageDuration: Math.max(0.1, duration),
          },
        ],
        selectedClipIndex: state.clips.length,
      };
    }),

  setImageDuration: (index, duration) =>
    set((state) => {
      const clip = state.clips[index];
      if (clip?.type !== 'image') return state;
      const undo = pushUndo(state);
      const clips = [...state.clips];
      clips[index] = { ...clips[index], imageDuration: Math.max(0.1, duration) };
      return { ...undo, clips };
    }),

  selectClip: (index) => set({ selectedClipIndex: index, selectedEffectId: null }),
  setEditedVideoPath: (path) => set({ editedVideoPath: path }),
  setEditedVideoPlanHash: (hash) => set({ editedVideoPlanHash: hash }),

  getOutputDuration: () => get().clips.reduce((sum, c) => sum + clipOutputDuration(c), 0),

  getClipOutputStart: (index) => {
    const clips = get().clips;
    let start = 0;
    for (let i = 0; i < index && i < clips.length; i++) {
      start += clipOutputDuration(clips[i]);
    }
    return start;
  },

  resolveAtOutputTime: (outputTime) => {
    const state = get();
    const clips = state.clips;
    if (clips.length === 0) return null;
    const total = clips.reduce((s, c) => s + clipOutputDuration(c), 0);
    // Clamp first so the last clip is reachable even when the caller passes
    // a slightly-past-end outputTime (rAF drift, float accumulation).
    const t = Math.max(0, Math.min(total, outputTime));
    let cumulative = 0;
    for (let i = 0; i < clips.length; i++) {
      const clip = clips[i];
      const dur = clipOutputDuration(clip);
      const end = cumulative + dur;
      // Use `<` for the upper bound except on the last clip, where we include
      // the endpoint so the playhead at exactly `total` still resolves.
      const inside = i === clips.length - 1 ? t <= end : t < end;
      if (inside) {
        const localOutputTime = Math.max(0, Math.min(dur, t - cumulative));
        let sourceTime: number;
        if (clip.type === 'freeze') {
          sourceTime = clip.freezeSourceTime ?? clip.sourceStart;
        } else if (clip.type === 'image') {
          sourceTime = 0;
        } else {
          // Per-clip mapping — scoped to this clip's own source range, so
          // overlapping ranges from different MediaRefs never collide.
          sourceTime = clip.sourceStart + localOutputTime * clip.speed;
        }
        const refId = clip.mediaRefId ?? state.primaryMediaRefId;
        const mediaRef = refId ? state.mediaPool[refId] ?? null : null;
        return {
          clipIndex: i,
          clip,
          mediaRef,
          localOutputTime,
          sourceTime,
          clipOutputStart: cumulative,
          clipOutputDuration: dur,
        };
      }
      cumulative = end;
    }
    return null;
  },

  outputTimeToSource: (outputTime) => {
    const r = get().resolveAtOutputTime(outputTime);
    return r ? r.sourceTime : 0;
  },

  // ── Freeze Frame ──

  insertFreezeFrame: (outputTime, duration = 3.0) =>
    set((state) => {
      const undo = pushUndo(state);
      let cumulative = 0;
      for (let i = 0; i < state.clips.length; i++) {
        const clip = state.clips[i];
        const dur = clipOutputDuration(clip);
        if (outputTime >= cumulative && outputTime <= cumulative + dur) {
          const sourceTime = clip.type === 'freeze'
            ? (clip.freezeSourceTime ?? clip.sourceStart)
            : clip.sourceStart + (outputTime - cumulative) * clip.speed;

          const freezeClip: EditClip = {
            id: crypto.randomUUID(),
            mediaRefId: clip.mediaRefId,
            sourceStart: sourceTime,
            sourceEnd: sourceTime,
            speed: 1.0,
            skipFrames: false,
            fpsOverride: null,
            type: 'freeze',
            freezeSourceTime: sourceTime,
            freezeDuration: duration,
          };

          const clips = [...state.clips];
          // If we're inside a normal clip, split it and insert the freeze between halves
          if (clip.type !== 'freeze' && outputTime > cumulative && outputTime < cumulative + dur) {
            const relativeTime = (outputTime - cumulative) * clip.speed;
            const sourceMiddle = clip.sourceStart + relativeTime;
            const left: EditClip = { ...clip, id: crypto.randomUUID(), sourceEnd: sourceMiddle, zoomPan: clip.zoomPan ? structuredClone(clip.zoomPan) : undefined };
            const right: EditClip = { ...clip, id: crypto.randomUUID(), sourceStart: sourceMiddle, zoomPan: clip.zoomPan ? structuredClone(clip.zoomPan) : undefined };
            clips.splice(i, 1, left, freezeClip, right);
            return { ...undo, clips, selectedClipIndex: i + 1 };
          }

          // At clip boundary or on a freeze clip — insert after
          clips.splice(i + 1, 0, freezeClip);
          return { ...undo, clips, selectedClipIndex: i + 1 };
        }
        cumulative += dur;
      }
      // Past end — insert freeze at the last source time
      if (state.clips.length > 0) {
        const lastClip = state.clips[state.clips.length - 1];
        const sourceTime = lastClip.type === 'freeze' ? (lastClip.freezeSourceTime ?? lastClip.sourceEnd) : lastClip.sourceEnd;
        const freezeClip: EditClip = {
          id: crypto.randomUUID(),
          mediaRefId: lastClip.mediaRefId,
          sourceStart: sourceTime,
          sourceEnd: sourceTime,
          speed: 1.0,
          skipFrames: false,
          fpsOverride: null,
          type: 'freeze',
          freezeSourceTime: sourceTime,
          freezeDuration: duration,
        };
        return { ...undo, clips: [...state.clips, freezeClip], selectedClipIndex: state.clips.length };
      }
      return state;
    }),

  setFreezeDuration: (index, duration) =>
    set((state) => {
      const clip = state.clips[index];
      if (clip?.type !== 'freeze') return state;
      const undo = pushUndo(state);
      const clips = [...state.clips];
      clips[index] = { ...clips[index], freezeDuration: Math.max(0.1, duration) };
      return { ...undo, clips };
    }),

  // ── Zoom/Pan ──

  setClipZoomPan: (index, zoomPan) =>
    set((state) => {
      const undo = pushUndo(state);
      const clips = [...state.clips];
      clips[index] = { ...clips[index], zoomPan: zoomPan ? { ...zoomPan, startRegion: { ...zoomPan.startRegion }, endRegion: { ...zoomPan.endRegion } } : null };
      return { ...undo, clips };
    }),

  setClipZoomPanLive: (index, zoomPan) =>
    set((state) => {
      const snapshot = state._preZoomPanSnapshot || snapshotState(state);
      const clips = [...state.clips];
      clips[index] = { ...clips[index], zoomPan: { ...zoomPan, startRegion: { ...zoomPan.startRegion }, endRegion: { ...zoomPan.endRegion } } };
      return { clips, _preZoomPanSnapshot: snapshot };
    }),

  commitZoomPanChange: () =>
    set((state) => {
      if (!state._preZoomPanSnapshot) return state;
      const stack = [...state.undoStack, state._preZoomPanSnapshot];
      if (stack.length > MAX_UNDO) stack.shift();
      return { undoStack: stack, redoStack: [], _preZoomPanSnapshot: null };
    }),

  setClipEasing: (index, easing) =>
    set((state) => {
      const clip = state.clips[index];
      if (!clip?.zoomPan) return state;
      const undo = pushUndo(state);
      const clips = [...state.clips];
      clips[index] = { ...clips[index], zoomPan: { ...clip.zoomPan!, easing } };
      return { ...undo, clips };
    }),

  // ── Effects Track ──

  addEffect: (effect) =>
    set((state) => {
      if (effect.startTime >= effect.endTime) return state; // reject invalid
      const undo = pushUndo(state);
      const newEffect: TimelineEffect = { ...effect, id: crypto.randomUUID() };
      return { ...undo, effects: [...state.effects, newEffect], selectedEffectId: newEffect.id };
    }),

  removeEffect: (id) =>
    set((state) => {
      const undo = pushUndo(state);
      return { ...undo, effects: state.effects.filter((e) => e.id !== id), selectedEffectId: state.selectedEffectId === id ? null : state.selectedEffectId };
    }),

  updateEffect: (id, partial) =>
    set((state) => {
      const undo = pushUndo(state);
      const effects = state.effects.map((e) => {
        if (e.id !== id) return e;
        const merged = { ...e, ...partial };
        // Validate: swap if startTime >= endTime
        if (merged.startTime >= merged.endTime) {
          merged.endTime = Math.max(merged.startTime + 0.5, e.endTime);
        }
        return merged;
      });
      return { ...undo, effects };
    }),

  updateEffectLive: (id, partial) =>
    set((state) => {
      const snapshot = state._preEffectSnapshot || snapshotState(state);
      const effects = state.effects.map((e) => {
        if (e.id !== id) return e;
        const merged = { ...e, ...partial };
        // Prevent the preview from rendering an inverted-range effect during
        // live drag. If the user drags one edge past the other, clamp to a
        // minimum 0.1s window — matching the shape commit() would produce.
        if (merged.startTime >= merged.endTime) {
          if (partial.startTime !== undefined) {
            merged.startTime = Math.max(0, merged.endTime - 0.1);
          } else if (partial.endTime !== undefined) {
            merged.endTime = merged.startTime + 0.1;
          } else {
            return e; // neither bound changed — ignore this partial
          }
        }
        return merged;
      });
      return { effects, _preEffectSnapshot: snapshot };
    }),

  commitEffectChange: () =>
    set((state) => {
      if (!state._preEffectSnapshot) return state;
      const stack = [...state.undoStack, state._preEffectSnapshot];
      if (stack.length > MAX_UNDO) stack.shift();
      return { undoStack: stack, redoStack: [], _preEffectSnapshot: null };
    }),

  selectEffect: (id) => set((state) => ({ selectedEffectId: id, selectedClipIndex: id ? null : state.selectedClipIndex })),

  // Half-open interval [start, end) — avoids double-triggering at seams between adjacent effects
  getEffectsAtTime: (outputTime) => get().effects.filter((e) => outputTime >= e.startTime && outputTime < e.endTime),

  reset: () => set({ mediaPool: {}, primaryMediaRefId: null, clips: [], effects: [], selectedClipIndex: null, selectedEffectId: null, editedVideoPath: null, editedVideoPlanHash: null, sourceDuration: 0, undoStack: [], redoStack: [], _preSpeedSnapshot: null, _preZoomPanSnapshot: null, _preEffectSnapshot: null }),
}));
