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
  text?: { content: string; x: number; y: number; fontSize: number; color: string; fontFamily?: string; bold?: boolean; italic?: boolean; underline?: boolean; background?: string; align?: 'left' | 'center' | 'right' };
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

// ── Clip Types ──

export interface EditClip {
  id: string;
  sourceStart: number;
  sourceEnd: number;
  speed: number;
  skipFrames: boolean;
  fpsOverride: number | null;
  type?: 'normal' | 'freeze';
  freezeSourceTime?: number;
  freezeDuration?: number;
  zoomPan?: ZoomPanEffect | null;  // kept for backward compat with old saved projects
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
  return (c.sourceEnd - c.sourceStart) / c.speed;
}

interface EditStore {
  clips: EditClip[];
  effects: TimelineEffect[];
  selectedClipIndex: number | null;
  selectedEffectId: string | null;
  editedVideoPath: string | null;
  sourceDuration: number;

  // Undo/Redo
  undoStack: ClipSnapshot[];
  redoStack: ClipSnapshot[];
  _preSpeedSnapshot: ClipSnapshot | null;
  _preZoomPanSnapshot: ClipSnapshot | null;
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
  addClip: (sourceFile: string, sourceStart: number, sourceEnd: number) => void;
  selectClip: (index: number | null) => void;
  setEditedVideoPath: (path: string | null) => void;
  getOutputDuration: () => number;
  getClipOutputStart: (index: number) => number;
  outputTimeToSource: (outputTime: number) => number;
  reset: () => void;

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

function deepCopyEffect(e: TimelineEffect): TimelineEffect {
  return {
    ...e,
    zoomPan: e.zoomPan ? { ...e.zoomPan, startRegion: { ...e.zoomPan.startRegion }, endRegion: { ...e.zoomPan.endRegion } } : undefined,
    spotlight: e.spotlight ? { ...e.spotlight } : undefined,
    blur: e.blur ? { ...e.blur } : undefined,
    text: e.text ? { ...e.text } : undefined,
    fade: e.fade ? { ...e.fade } : undefined,
  };
}

function deepCopyClip(c: EditClip): EditClip {
  return { ...c, zoomPan: c.zoomPan ? { ...c.zoomPan, startRegion: { ...c.zoomPan.startRegion }, endRegion: { ...c.zoomPan.endRegion } } : c.zoomPan };
}

// Helper: push current state to undo stack before mutation
function pushUndo(state: EditStore): Partial<EditStore> {
  const snapshot: ClipSnapshot = {
    clips: state.clips.map(deepCopyClip),
    effects: state.effects.map(deepCopyEffect),
    selectedClipIndex: state.selectedClipIndex,
    selectedEffectId: state.selectedEffectId,
  };
  const stack = [...state.undoStack, snapshot];
  if (stack.length > MAX_UNDO) stack.shift();
  return { undoStack: stack, redoStack: [] };
}

function snapshotState(state: EditStore): ClipSnapshot {
  return {
    clips: state.clips.map(deepCopyClip),
    effects: state.effects.map(deepCopyEffect),
    selectedClipIndex: state.selectedClipIndex,
    selectedEffectId: state.selectedEffectId,
  };
}

export const useEditStore = create<EditStore>((set, get) => ({
  clips: [],
  effects: [],
  selectedClipIndex: null,
  selectedEffectId: null,
  editedVideoPath: null,
  sourceDuration: 0,
  undoStack: [],
  redoStack: [],
  _preSpeedSnapshot: null,
  _preZoomPanSnapshot: null,

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
    set({
      clips: [{ id: crypto.randomUUID(), sourceStart: 0, sourceEnd: duration, speed: 1.0, skipFrames: false, fpsOverride: null }],
      effects: [],
      selectedClipIndex: 0,
      selectedEffectId: null,
      editedVideoPath: null,
      sourceDuration: duration,
      undoStack: [],
      redoStack: [],
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
          const left: EditClip = { ...clip, id: crypto.randomUUID(), sourceEnd: sourceMiddle, zoomPan: null };
          const right: EditClip = { ...clip, id: crypto.randomUUID(), sourceStart: sourceMiddle, zoomPan: null };
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

  addClip: (_sourceFile, sourceStart, sourceEnd) =>
    set((state) => {
      const undo = pushUndo(state);
      return {
        ...undo,
        clips: [...state.clips, { id: crypto.randomUUID(), sourceStart, sourceEnd, speed: 1.0, skipFrames: false, fpsOverride: null }],
        selectedClipIndex: state.clips.length,
      };
    }),

  selectClip: (index) => set({ selectedClipIndex: index, selectedEffectId: null }),
  setEditedVideoPath: (path) => set({ editedVideoPath: path }),

  getOutputDuration: () => get().clips.reduce((sum, c) => sum + clipOutputDuration(c), 0),

  getClipOutputStart: (index) => {
    const clips = get().clips;
    let start = 0;
    for (let i = 0; i < index && i < clips.length; i++) {
      start += clipOutputDuration(clips[i]);
    }
    return start;
  },

  outputTimeToSource: (outputTime) => {
    const clips = get().clips;
    let cumulative = 0;
    for (const clip of clips) {
      const dur = clipOutputDuration(clip);
      if (outputTime <= cumulative + dur) {
        if (clip.type === 'freeze') return clip.freezeSourceTime ?? clip.sourceStart;
        return clip.sourceStart + (outputTime - cumulative) * clip.speed;
      }
      cumulative += dur;
    }
    return clips.length > 0 ? clips[clips.length - 1].sourceEnd : 0;
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
            const left: EditClip = { ...clip, id: crypto.randomUUID(), sourceEnd: sourceMiddle, zoomPan: null };
            const right: EditClip = { ...clip, id: crypto.randomUUID(), sourceStart: sourceMiddle, zoomPan: null };
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
      const effects = state.effects.map((e) => e.id === id ? { ...e, ...partial } : e);
      return { ...undo, effects };
    }),

  updateEffectLive: (id, partial) =>
    set((state) => {
      const snapshot = state._preZoomPanSnapshot || snapshotState(state);
      const effects = state.effects.map((e) => e.id === id ? { ...e, ...partial } : e);
      return { effects, _preZoomPanSnapshot: snapshot };
    }),

  commitEffectChange: () =>
    set((state) => {
      if (!state._preZoomPanSnapshot) return state;
      const stack = [...state.undoStack, state._preZoomPanSnapshot];
      if (stack.length > MAX_UNDO) stack.shift();
      return { undoStack: stack, redoStack: [], _preZoomPanSnapshot: null };
    }),

  selectEffect: (id) => set((state) => ({ selectedEffectId: id, selectedClipIndex: id ? null : state.selectedClipIndex })),

  getEffectsAtTime: (outputTime) => get().effects.filter((e) => outputTime >= e.startTime && outputTime <= e.endTime),

  reset: () => set({ clips: [], effects: [], selectedClipIndex: null, selectedEffectId: null, editedVideoPath: null, sourceDuration: 0, undoStack: [], redoStack: [], _preSpeedSnapshot: null, _preZoomPanSnapshot: null }),
}));
