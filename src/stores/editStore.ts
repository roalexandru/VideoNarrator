import { create } from "zustand";

export interface EditClip {
  id: string;
  sourceStart: number;
  sourceEnd: number;
  speed: number;
  skipFrames: boolean;
  fpsOverride: number | null;
}

// Snapshot for undo/redo — only the data that changes
interface ClipSnapshot {
  clips: EditClip[];
  selectedClipIndex: number | null;
}

const MAX_UNDO = 30;

interface EditStore {
  clips: EditClip[];
  selectedClipIndex: number | null;
  editedVideoPath: string | null;
  sourceDuration: number;

  // Undo/Redo
  undoStack: ClipSnapshot[];
  redoStack: ClipSnapshot[];
  canUndo: () => boolean;
  canRedo: () => boolean;
  undo: () => void;
  redo: () => void;

  initFromVideo: (duration: number) => void;
  splitAt: (outputTime: number) => void;
  deleteClip: (index: number) => void;
  setClipSpeed: (index: number, speed: number) => void;
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
}

// Helper: push current state to undo stack before mutation
function pushUndo(state: EditStore): Partial<EditStore> {
  const snapshot: ClipSnapshot = {
    clips: state.clips.map((c) => ({ ...c })),
    selectedClipIndex: state.selectedClipIndex,
  };
  const stack = [...state.undoStack, snapshot];
  if (stack.length > MAX_UNDO) stack.shift();
  return { undoStack: stack, redoStack: [] }; // clear redo on new action
}

export const useEditStore = create<EditStore>((set, get) => ({
  clips: [],
  selectedClipIndex: null,
  editedVideoPath: null,
  sourceDuration: 0,
  undoStack: [],
  redoStack: [],

  canUndo: () => get().undoStack.length > 0,
  canRedo: () => get().redoStack.length > 0,

  undo: () =>
    set((state) => {
      if (state.undoStack.length === 0) return state;
      const stack = [...state.undoStack];
      const prev = stack.pop()!;
      // Push current to redo
      const redoSnapshot: ClipSnapshot = { clips: state.clips.map((c) => ({ ...c })), selectedClipIndex: state.selectedClipIndex };
      return { ...prev, undoStack: stack, redoStack: [...state.redoStack, redoSnapshot] };
    }),

  redo: () =>
    set((state) => {
      if (state.redoStack.length === 0) return state;
      const stack = [...state.redoStack];
      const next = stack.pop()!;
      // Push current to undo
      const undoSnapshot: ClipSnapshot = { clips: state.clips.map((c) => ({ ...c })), selectedClipIndex: state.selectedClipIndex };
      return { ...next, redoStack: stack, undoStack: [...state.undoStack, undoSnapshot] };
    }),

  initFromVideo: (duration) =>
    set({
      clips: [{ id: crypto.randomUUID(), sourceStart: 0, sourceEnd: duration, speed: 1.0, skipFrames: false, fpsOverride: null }],
      selectedClipIndex: 0,
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
        const clipOutputDur = (clip.sourceEnd - clip.sourceStart) / clip.speed;
        if (outputTime > cumulative && outputTime < cumulative + clipOutputDur) {
          const undo = pushUndo(state);
          const relativeTime = (outputTime - cumulative) * clip.speed;
          const sourceMiddle = clip.sourceStart + relativeTime;
          const left: EditClip = { ...clip, id: crypto.randomUUID(), sourceEnd: sourceMiddle };
          const right: EditClip = { ...clip, id: crypto.randomUUID(), sourceStart: sourceMiddle };
          const clips = [...state.clips];
          clips.splice(i, 1, left, right);
          return { ...undo, clips, selectedClipIndex: i };
        }
        cumulative += clipOutputDur;
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
      const undo = pushUndo(state);
      const clips = [...state.clips];
      clips[index] = { ...clips[index], speed };
      return { ...undo, clips };
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

  selectClip: (index) => set({ selectedClipIndex: index }),
  setEditedVideoPath: (path) => set({ editedVideoPath: path }),

  getOutputDuration: () => get().clips.reduce((sum, c) => sum + (c.sourceEnd - c.sourceStart) / c.speed, 0),

  getClipOutputStart: (index) => {
    const clips = get().clips;
    let start = 0;
    for (let i = 0; i < index && i < clips.length; i++) {
      start += (clips[i].sourceEnd - clips[i].sourceStart) / clips[i].speed;
    }
    return start;
  },

  outputTimeToSource: (outputTime) => {
    const clips = get().clips;
    let cumulative = 0;
    for (const clip of clips) {
      const clipOutputDur = (clip.sourceEnd - clip.sourceStart) / clip.speed;
      if (outputTime <= cumulative + clipOutputDur) {
        return clip.sourceStart + (outputTime - cumulative) * clip.speed;
      }
      cumulative += clipOutputDur;
    }
    return clips.length > 0 ? clips[clips.length - 1].sourceEnd : 0;
  },

  reset: () => set({ clips: [], selectedClipIndex: null, editedVideoPath: null, sourceDuration: 0, undoStack: [], redoStack: [] }),
}));
