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

/** Models offered in the picker (current generation). */
export type CurrentModelId =
  // Anthropic
  | "claude-fable-5"
  | "claude-opus-5"
  | "claude-sonnet-5"
  | "claude-haiku-4-5"
  // OpenAI — GPT-5.6 ships as three named variants
  | "gpt-5.6-sol"
  | "gpt-5.6-terra"
  | "gpt-5.6-luna"
  // Google
  | "gemini-3.1-pro-preview"
  | "gemini-3.6-flash"
  | "gemini-3.5-flash"
  | "gemini-3.5-flash-lite";

/**
 * Model IDs that may appear in a *saved* project but are no longer offered.
 * Kept in the union so loading an older project still type-checks; the picker
 * only lists `CurrentModelId`.
 */
export type LegacyModelId =
  | "claude-sonnet-4-20250514"
  | "claude-opus-4-20250514"
  | "gpt-4o"
  | "o3"
  | "gemini-2.5-flash"
  | "gemini-2.5-pro";

export type ModelId = CurrentModelId | LegacyModelId;

/**
 * Provider-agnostic reasoning depth. Mapped to each vendor's own parameter in
 * the Rust client (`output_config.effort` / `reasoning_effort` / `thinkingLevel`).
 */
export type ReasoningEffort = "fast" | "balanced" | "thorough" | "max";

export type TtsProvider = "elevenlabs" | "azure" | "builtin";

export interface TtsProviderKeyStatus {
  provider: TtsProvider;
  has_key: boolean;
}

export interface AiConfig {
  provider: AiProvider;
  model: ModelId;
  temperature: number;
  reasoning_effort: ReasoningEffort;
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
  /** When true, run an extra critique+refine pass after generation so
   *  segments whose narration doesn't match the frames get rewritten. Costs
   *  one multimodal call plus up to five refine calls per iteration. */
  strict_mode?: boolean;
  /** Send frames as tiled contact sheets instead of one image per frame. Cuts
   *  API calls ~9x but halves each frame's linear resolution — left off by
   *  default because OCR only compensates for that on macOS. */
  use_contact_sheets?: boolean;
  /** Let the model choose which moments get full-resolution frames, via a cheap
   *  low-resolution survey pass. Falls back to even spacing on any failure. */
  use_model_frame_selection?: boolean;
  /** Read on-screen text (OCR) and add it to the prompt, so narration can name
   *  the actual command, file or error. Inert where no recognizer exists. */
  use_screen_text?: boolean;
}
