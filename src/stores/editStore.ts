import { create } from "zustand";

export interface EditClip {
  id: string;
  sourceStart: number;
  sourceEnd: number;
  speed: number;
  skipFrames: boolean;  // true = drop frames (clean cuts), false = speed up (fast-forward)
  fpsOverride: number | null;
}

interface EditStore {
  clips: EditClip[];
  selectedClipIndex: number | null;
  editedVideoPath: string | null;
  sourceDuration: number; // original video duration

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
  // Convert output timeline position to source position for video seeking
  outputTimeToSource: (outputTime: number) => number;
  reset: () => void;
}

export const useEditStore = create<EditStore>((set, get) => ({
  clips: [],
  selectedClipIndex: null,
  editedVideoPath: null,
  sourceDuration: 0,

  initFromVideo: (duration) =>
    set({
      clips: [{ id: crypto.randomUUID(), sourceStart: 0, sourceEnd: duration, speed: 1.0, skipFrames: false, fpsOverride: null }],
      selectedClipIndex: 0,
      editedVideoPath: null,
      sourceDuration: duration,
    }),

  splitAt: (outputTime) =>
    set((state) => {
      // Find which clip contains this output time
      let cumulative = 0;
      for (let i = 0; i < state.clips.length; i++) {
        const clip = state.clips[i];
        const clipOutputDur = (clip.sourceEnd - clip.sourceStart) / clip.speed;
        if (outputTime > cumulative && outputTime < cumulative + clipOutputDur) {
          // Split this clip at the relative position
          const relativeTime = (outputTime - cumulative) * clip.speed; // in source time
          const sourceMiddle = clip.sourceStart + relativeTime;
          const left: EditClip = { ...clip, id: crypto.randomUUID(), sourceEnd: sourceMiddle };
          const right: EditClip = { ...clip, id: crypto.randomUUID(), sourceStart: sourceMiddle };
          const clips = [...state.clips];
          clips.splice(i, 1, left, right);
          return { clips, selectedClipIndex: i };
        }
        cumulative += clipOutputDur;
      }
      return state;
    }),

  deleteClip: (index) =>
    set((state) => {
      if (state.clips.length <= 1) return state;
      const clips = state.clips.filter((_, i) => i !== index);
      return { clips, selectedClipIndex: clips.length > 0 ? Math.min(index, clips.length - 1) : null };
    }),

  setClipSpeed: (index, speed) =>
    set((state) => {
      const clips = [...state.clips];
      clips[index] = { ...clips[index], speed };
      return { clips };
    }),

  setClipSkipFrames: (index, skip) =>
    set((state) => {
      const clips = [...state.clips];
      clips[index] = { ...clips[index], skipFrames: skip };
      return { clips };
    }),

  setClipFps: (index, fps) =>
    set((state) => {
      const clips = [...state.clips];
      clips[index] = { ...clips[index], fpsOverride: fps };
      return { clips };
    }),

  moveClip: (fromIndex, toIndex) =>
    set((state) => {
      if (fromIndex === toIndex) return state;
      const clips = [...state.clips];
      const [moved] = clips.splice(fromIndex, 1);
      clips.splice(toIndex, 0, moved);
      return { clips, selectedClipIndex: toIndex };
    }),

  addClip: (_sourceFile, sourceStart, sourceEnd) =>
    set((state) => ({
      clips: [...state.clips, { id: crypto.randomUUID(), sourceStart, sourceEnd, speed: 1.0, skipFrames: false, fpsOverride: null }],
      selectedClipIndex: state.clips.length,
    })),

  selectClip: (index) => set({ selectedClipIndex: index }),
  setEditedVideoPath: (path) => set({ editedVideoPath: path }),

  getOutputDuration: () => {
    return get().clips.reduce((sum, c) => sum + (c.sourceEnd - c.sourceStart) / c.speed, 0);
  },

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
        const relativeOutput = outputTime - cumulative;
        return clip.sourceStart + relativeOutput * clip.speed;
      }
      cumulative += clipOutputDur;
    }
    // Past the end — return last clip's end
    return clips.length > 0 ? clips[clips.length - 1].sourceEnd : 0;
  },

  reset: () => set({ clips: [], selectedClipIndex: null, editedVideoPath: null, sourceDuration: 0 }),
}));
