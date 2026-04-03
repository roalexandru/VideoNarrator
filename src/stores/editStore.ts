import { create } from "zustand";

export interface EditClip {
  id: string;
  startSeconds: number;
  endSeconds: number;
  speed: number;
  fpsOverride: number | null;
}

interface EditStore {
  clips: EditClip[];
  selectedClipIndex: number | null;
  editedVideoPath: string | null;

  initFromVideo: (duration: number) => void;
  splitAt: (seconds: number) => void;
  deleteClip: (index: number) => void;
  setClipSpeed: (index: number, speed: number) => void;
  setClipFps: (index: number, fps: number | null) => void;
  trimClipStart: (index: number, newStart: number) => void;
  trimClipEnd: (index: number, newEnd: number) => void;
  selectClip: (index: number | null) => void;
  setEditedVideoPath: (path: string | null) => void;
  reset: () => void;
}

export const useEditStore = create<EditStore>((set) => ({
  clips: [],
  selectedClipIndex: null,
  editedVideoPath: null,

  initFromVideo: (duration) =>
    set({
      clips: [{ id: crypto.randomUUID(), startSeconds: 0, endSeconds: duration, speed: 1.0, fpsOverride: null }],
      selectedClipIndex: 0,
      editedVideoPath: null,
    }),

  splitAt: (seconds) =>
    set((state) => {
      const idx = state.clips.findIndex((c) => seconds > c.startSeconds && seconds < c.endSeconds);
      if (idx < 0) return state;
      const clip = state.clips[idx];
      const left: EditClip = { ...clip, id: crypto.randomUUID(), endSeconds: seconds };
      const right: EditClip = { ...clip, id: crypto.randomUUID(), startSeconds: seconds };
      const clips = [...state.clips];
      clips.splice(idx, 1, left, right);
      return { clips, selectedClipIndex: idx };
    }),

  deleteClip: (index) =>
    set((state) => {
      const clips = state.clips.filter((_, i) => i !== index);
      return { clips, selectedClipIndex: clips.length > 0 ? Math.min(index, clips.length - 1) : null };
    }),

  setClipSpeed: (index, speed) =>
    set((state) => {
      const clips = [...state.clips];
      clips[index] = { ...clips[index], speed };
      return { clips };
    }),

  setClipFps: (index, fps) =>
    set((state) => {
      const clips = [...state.clips];
      clips[index] = { ...clips[index], fpsOverride: fps };
      return { clips };
    }),

  trimClipStart: (index, newStart) =>
    set((state) => {
      const clips = [...state.clips];
      clips[index] = { ...clips[index], startSeconds: Math.max(0, newStart) };
      return { clips };
    }),

  trimClipEnd: (index, newEnd) =>
    set((state) => {
      const clips = [...state.clips];
      clips[index] = { ...clips[index], endSeconds: newEnd };
      return { clips };
    }),

  selectClip: (index) => set({ selectedClipIndex: index }),
  setEditedVideoPath: (path) => set({ editedVideoPath: path }),
  reset: () => set({ clips: [], selectedClipIndex: null, editedVideoPath: null }),
}));
