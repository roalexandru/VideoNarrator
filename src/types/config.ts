export type NarrationStyleId =
  | "executive"
  | "product_demo"
  | "technical"
  | "teaser"
  | "training"
  | "critique";

export interface NarrationStyleDef {
  id: NarrationStyleId;
  label: string;
  description: string;
  system_prompt: string;
  pacing: string;
  pause_markers: boolean;
}

export type LanguageCode = "en" | "ja" | "de" | "fr" | "pt-BR" | string;

export interface Language {
  code: LanguageCode;
  label: string;
  flag: string;
}

export type FrameDensity = "light" | "medium" | "heavy";
export type AiProvider = "claude" | "openai" | "gemini";
export type ModelId =
  | "claude-sonnet-4-20250514"
  | "claude-opus-4-20250514"
  | "gpt-4o"
  | "o3"
  | "gemini-2.5-flash"
  | "gemini-2.5-pro";

export type TtsProvider = "elevenlabs" | "azure" | "builtin";

export interface TtsProviderKeyStatus {
  provider: TtsProvider;
  has_key: boolean;
}

export interface AiConfig {
  provider: AiProvider;
  model: ModelId;
  temperature: number;
}

export interface FrameConfig {
  density: FrameDensity;
  scene_threshold: number;
  max_frames: number;
  skip_dedup?: boolean;
}

export interface ProviderKeyStatus {
  provider: AiProvider;
  has_key: boolean;
  models: string[];
}

export interface GenerationParams {
  project_id: string;
  video_path: string;
  document_paths: string[];
  title: string;
  description: string;
  style: string;
  primary_language: string;
  additional_languages: string[];
  frame_config: FrameConfig;
  ai_config: AiConfig;
  custom_prompt: string;
  /** Segments from a prior partial run — if provided, generation resumes
   *  after the last segment's end_seconds rather than re-consuming API calls
   *  for chunks that already succeeded. */
  resume_segments?: import("./script").Segment[];
}
