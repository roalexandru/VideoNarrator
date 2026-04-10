export interface NarrationScript {
  title: string;
  total_duration_seconds: number;
  segments: Segment[];
  metadata: ScriptMetadata;
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
