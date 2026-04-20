import type { Segment } from "./script";

export interface Frame {
  index: number;
  timestamp_seconds: number;
  path: string;
  width: number;
  height: number;
}

export type ProcessingPhase =
  | "idle"
  | "applying_edits"
  | "extracting_frames"
  | "processing_docs"
  | "generating_narration"
  | "done"
  | "error"
  | "cancelled";

export type ProgressEvent =
  | { kind: "phase_change"; phase: ProcessingPhase }
  /**
   * Monotonic percent update. `message` is an optional human-readable
   * sub-label shown under the progress bar (e.g. "Analyzing batch 2 of 5"),
   * omitted for intra-stage ticks that would only repeat the previous label.
   */
  | { kind: "progress"; percent: number; message?: string }
  | { kind: "frame_extracted"; frame: Frame }
  | { kind: "segment_streamed"; segment: Segment }
  /** Terminal event: the full normalized script. Replaces streaming preview. */
  | { kind: "segments_replaced"; segments: Segment[] }
  | { kind: "error"; message: string };
