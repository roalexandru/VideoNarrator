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
  /**
   * Final frame paths, sent once extraction's temp directory has been promoted
   * into the project. `frame_extracted` necessarily carries the temp path (it
   * fires while extraction runs) and that directory is renamed away, so without
   * this the thumbnails render blank.
   */
  | { kind: "frames_replaced"; frames: Frame[] }
  /** What grounding actually applied. Fields are absent when the feature no-op'd. */
  | {
      kind: "grounding";
      screen_text_screens?: number;
      model_selected_moments?: number;
    }
  /** Terminal event: the full normalized script. Replaces streaming preview. */
  | { kind: "segments_replaced"; segments: Segment[] }
  /**
   * Emitted once after an export finishes, reporting whether the rendered file
   * matches what was planned. Advisory: the export has already succeeded by the
   * time this arrives, so a failing check is information, never an error.
   */
  | { kind: "export_verified"; report: VerificationReport }
  | { kind: "error"; message: string };

/** One post-export check. `detail` is the human-readable result either way. */
export interface ExportCheck {
  id: string;
  label: string;
  status: "pass" | "fail" | "skipped";
  detail: string;
}

export interface VerificationReport {
  checks: ExportCheck[];
}
