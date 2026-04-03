import { create } from "zustand";
import type { Frame, ProcessingPhase } from "../types/processing";
import type { Segment } from "../types/script";

interface ProcessingStore {
  phase: ProcessingPhase;
  progress: number;
  frames: Frame[];
  streamingSegments: Segment[];
  error: string | null;

  setPhase: (phase: ProcessingPhase) => void;
  setProgress: (pct: number) => void;
  appendFrame: (frame: Frame) => void;
  appendSegment: (segment: Segment) => void;
  setError: (err: string | null) => void;
  reset: () => void;
}

export const useProcessingStore = create<ProcessingStore>((set) => ({
  phase: "idle",
  progress: 0,
  frames: [],
  streamingSegments: [],
  error: null,

  setPhase: (phase) => set({ phase }),
  setProgress: (progress) => set({ progress }),
  appendFrame: (frame) =>
    set((state) => ({ frames: [...state.frames, frame] })),
  appendSegment: (segment) =>
    set((state) => ({
      streamingSegments: [...state.streamingSegments, segment],
    })),
  setError: (error) => set({ error }),
  reset: () =>
    set({
      phase: "idle",
      progress: 0,
      frames: [],
      streamingSegments: [],
      error: null,
    }),
}));
