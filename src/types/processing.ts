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
  | "extracting_frames"
  | "processing_docs"
  | "generating_narration"
  | "done"
  | "error"
  | "cancelled";

export type ProgressEvent =
  | { kind: "phase_change"; phase: ProcessingPhase }
  | { kind: "progress"; percent: number }
  | { kind: "frame_extracted"; frame: Frame }
  | { kind: "segment_streamed"; segment: Segment }
  /** Terminal event: the full normalized script. Replaces streaming preview. */
  | { kind: "segments_replaced"; segments: Segment[] }
  | { kind: "error"; message: string };
