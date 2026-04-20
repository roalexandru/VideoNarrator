import { create } from "zustand";
import type { Frame, ProcessingPhase } from "../types/processing";
import type { Segment } from "../types/script";

interface ProcessingStore {
  phase: ProcessingPhase;
  progress: number;
  /** Live sub-label shown under the progress bar ("Analyzing batch 2 of 5",
   *  "Processing clip 1 of 3"). Cleared on reset. `null` means the UI should
   *  fall back to the generic phase label. */
  statusMessage: string | null;
  frames: Frame[];
  streamingSegments: Segment[];
  error: string | null;

  setPhase: (phase: ProcessingPhase) => void;
  /** Set progress percent. Monotonic-forward: a smaller value is ignored
   *  UNLESS it's 0, which is an explicit reset (used by the resume flow). */
  setProgress: (pct: number) => void;
  setStatusMessage: (msg: string | null) => void;
  appendFrame: (frame: Frame) => void;
  appendSegment: (segment: Segment) => void;
  /** Replace the streaming preview with the final, normalized segment list.
   *  Used for the terminal `segments_replaced` progress event so the UI
   *  reflects post-processing (polish, merge, dedup) without duplicates. */
  replaceStreamingSegments: (segments: Segment[]) => void;
  setError: (err: string | null) => void;
  reset: () => void;
}

export const useProcessingStore = create<ProcessingStore>((set) => ({
  phase: "idle",
  progress: 0,
  statusMessage: null,
  frames: [],
  streamingSegments: [],
  error: null,

  setPhase: (phase) => set({ phase }),
  setProgress: (progress) =>
    set((state) => {
      // `progress === 0` is an explicit reset (e.g. retry flow zeroes the
      // bar before the next run). Anything > 0 is clamped monotonic-forward
      // so out-of-order events from concurrent channels (edit → main) can
      // never walk the bar backward.
      if (progress <= 0) return { progress: 0 };
      return { progress: Math.max(state.progress, progress) };
    }),
  setStatusMessage: (statusMessage) => set({ statusMessage }),
  appendFrame: (frame) =>
    set((state) => ({ frames: [...state.frames, frame] })),
  appendSegment: (segment) =>
    set((state) => ({
      streamingSegments: [...state.streamingSegments, segment],
    })),
  replaceStreamingSegments: (segments) => set({ streamingSegments: segments }),
  setError: (error) => set({ error }),
  reset: () =>
    set({
      phase: "idle",
      progress: 0,
      statusMessage: null,
      frames: [],
      streamingSegments: [],
      error: null,
    }),
}));
