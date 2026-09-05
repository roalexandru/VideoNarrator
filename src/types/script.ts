export interface NarrationScript {
  title: string;
  total_duration_seconds: number;
  segments: Segment[];
  metadata: ScriptMetadata;
  /**
   * Per-segment speech-rate prediction produced by the Rust-side validator at
   * generation time. Optional — may be absent on scripts generated before this
   * field existed, or while the user is mid-edit (the Review screen recomputes
   * it client-side from `segments[i].text` during edits).
   */
  speech_rate_report?: import("../lib/speechRate").SegmentOverflow[];
  /**
   * Named sections over the narration. Absent means chapters were never
   * generated; an empty array means the model was asked and judged the video
   * too short or too uniform to divide.
   */
  chapters?: Chapter[];
}

/**
 * One named section of the narration. A chapter is identified only by where it
 * starts — its end is the next chapter's start — so gaps and overlaps are
 * unrepresentable.
 */
export interface Chapter {
  title: string;
  start_seconds: number;
  start_segment: number;
}

export interface Segment {
  index: number;
  start_seconds: number;
  end_seconds: number;
  text: string;
  visual_description: string;
  emphasis: string[];
  pace: "slow" | "medium" | "fast";
  pause_after_ms: number;
  frame_refs: number[];
  voice_override?: string;
}

export interface ScriptMetadata {
  style: string;
  language: string;
  provider: string;
  model: string;
  generated_at: string;
}
