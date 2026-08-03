import { invoke } from "@tauri-apps/api/core";
import { Channel } from "@tauri-apps/api/core";
import type { VideoMetadata } from "../../types/project";
import type {
  AiConfig,
  AiProvider,
  GenerationParams,
  ProviderKeyStatus,
} from "../../types/config";
import type { NarrationScript } from "../../types/script";
import type { NarrationStyleDef } from "../../types/config";
import type { ProgressEvent } from "../../types/processing";
import type { ExportOptions, ExportResult } from "../../types/export";

// System
export const checkFfmpeg = () => invoke<string>("check_ffmpeg");

/** True when the detected ffmpeg has the `subtitles` filter (libass). The
 *  frontend uses this to disable the "Burn subtitles" toggle — a libass-less
 *  ffmpeg silently produces a cryptic filter parse error at export time. */
export const ffmpegSupportsSubtitleBurn = () =>
  invoke<boolean>("ffmpeg_supports_subtitle_burn");

export const getProviderStatus = () =>
  invoke<ProviderKeyStatus[]>("get_provider_status");

export const setApiKey = (provider: AiProvider, key: string) =>
  invoke<void>("set_api_key", { provider, key });

export const validateApiKey = (provider: AiProvider, key: string) =>
  invoke<boolean>("validate_api_key_cmd", { provider, key });

// Video
export const probeVideo = (path: string) =>
  invoke<VideoMetadata>("probe_video", { path });

/**
 * Verify the app can actually read the file (surfaces macOS TCC denials
 * which otherwise manifest as a silent black preview).
 */
export const checkFileReadable = (path: string) =>
  invoke<boolean>("check_file_readable", { path });

/** Cheap existence check. Used by Export to decide whether the cached
 *  edited video needs regenerating. */
export const fileExists = (path: string) =>
  invoke<boolean>("file_exists", { path });

/** Fast content-aware fingerprint for a media file (blake3 over size +
 *  head + tail). Used by the media-pool dedupe to recognise that two paths
 *  point at the same underlying file. */
export const computeMediaHash = (path: string) =>
  invoke<string>("compute_media_hash", { path });

// Documents
export const processDocuments = (paths: string[]) =>
  invoke<{ name: string; content: string; token_estimate: number }[]>(
    "process_documents",
    { paths }
  );

// Generation
export const startGeneration = (
  params: GenerationParams,
  channel: Channel<ProgressEvent>
) => invoke<NarrationScript>("generate_narration", { params, channel });

export const cancelGeneration = () => invoke<void>("cancel_generation");

export const translateScript = (
  script: NarrationScript,
  targetLang: string,
  aiConfig: AiConfig
) =>
  invoke<NarrationScript>("translate_script", {
    script,
    targetLang,
    aiConfig,
  });

/** Rewrite one segment. Passing `projectId` records the instruction as a
 *  standing preference so future regenerations honour it. */
export const refineSegment = (
  segmentText: string,
  instruction: string,
  context: string,
  aiConfig: AiConfig,
  projectId?: string,
) =>
  invoke<string>("refine_segment", {
    segmentText,
    instruction,
    context,
    aiConfig,
    projectId,
  });

/** Rewrite the entire narration script with a user instruction.
 *  Preserves timestamps + style; stays grounded in visual descriptions.
 *  Passing `projectId` records the instruction as a standing preference. */
export const refineScript = (
  script: NarrationScript,
  instruction: string,
  aiConfig: AiConfig,
  styleHint?: string,
  customPrompt?: string,
  projectId?: string,
) =>
  invoke<NarrationScript>("refine_script", {
    script,
    instruction,
    aiConfig,
    styleHint,
    customPrompt,
    projectId,
  });

/** A refinement instruction retained as a standing preference for a project. */
export interface Preference {
  id: string;
  instruction: string;
  source: "script_refinement" | "segment_refinement" | "manual";
  active: boolean;
  created_at: string;
}

/** Standing narration preferences accumulated from earlier refinements. */
export const listPreferences = (projectId: string) =>
  invoke<Preference[]>("list_preferences", { projectId });

/** Toggle a preference without forgetting it, so it is not re-created when the
 *  user next phrases the same instruction. */
export const setPreferenceActive = (
  projectId: string,
  preferenceId: string,
  active: boolean,
) =>
  invoke<void>("set_preference_active", { projectId, preferenceId, active });

/** Delete a preference outright. */
export const deletePreference = (projectId: string, preferenceId: string) =>
  invoke<void>("delete_preference", { projectId, preferenceId });

// Projects
export const saveProject = (config: unknown) =>
  invoke<string>("save_project", { config });

export const loadProject = (id: string) =>
  invoke<unknown>("load_project", { id });

export interface ProjectSummary {
  id: string;
  title: string;
  video_path: string;
  style: string;
  created_at: string;
  updated_at: string;
  has_script: boolean;
  thumbnail_path: string | null;
  script_languages: string[];
}

export const listProjects = () => invoke<ProjectSummary[]>("list_projects");
export const deleteProject = (id: string) => invoke<void>("delete_project", { id });
export const exportProject = (id: string, outputPath: string) =>
  invoke<void>("export_project", { id, outputPath });
export const importProject = (archivePath: string) =>
  invoke<string>("import_project", { archivePath });

// Templates
export interface ProjectTemplate {
  id: string;
  name: string;
  style: string;
  languages: string[];
  primary_language: string;
  frame_config: { density: string; scene_threshold: number; max_frames: number };
  ai_config: AiConfig;
  custom_prompt: string;
  tts_provider: string;
  created_at: string;
}

export const saveTemplate = (template: ProjectTemplate) =>
  invoke<void>("save_template", { template });
export const listTemplates = () =>
  invoke<ProjectTemplate[]>("list_templates");
export const deleteTemplate = (id: string) =>
  invoke<void>("delete_template", { id });

export interface LoadedProject {
  config: {
    id: string;
    title: string;
    description: string;
    video_path: string;
    style: string;
    languages: string[];
    primary_language: string;
    frame_config: { density: string; scene_threshold: number; max_frames: number };
    ai_config: { provider: string; model: string; temperature: number; reasoning_effort?: import("../../types/config").ReasoningEffort };
    custom_prompt: string;
    created_at: string;
    updated_at: string;
    edit_clips?: {
      source_start: number; source_end: number; speed: number; skip_frames: boolean; fps_override: number | null;
      clip_type?: string; freeze_source_time?: number; freeze_duration?: number;
      /** Points into config.media_pool. Older projects omit this — callers resolve to the primary video. */
      media_ref_id?: string;
      /** For image clips: how long the still shows on the timeline. */
      image_duration?: number;
      zoom_pan?: { startRegion: { x: number; y: number; width: number; height: number }; endRegion: { x: number; y: number; width: number; height: number }; easing: string } | null;
    }[];
    /** Imported source files (videos + images added via "+"). Keyed by MediaRef.id.
     *  The primary video lives at key "primary" but we skip saving it here —
     *  it's reconstructed from video_metadata. */
    media_pool?: Record<string, {
      hash: string; kind: "video" | "image"; path: string;
      duration: number; width: number; height: number; fps?: number;
    }>;
    timeline_effects?: unknown[];
    video_metadata?: VideoMetadata;
    context_documents?: { id: string; path: string; name: string; size: number; type: string; tokenCount?: number }[];
    /**
     * Path to the cached edited video produced by the last applyVideoEdits call.
     * Export uses this as the source; if missing or the hash doesn't match
     * the current edit plan, Export regenerates it.
     */
    edited_video_path?: string;
    edited_video_plan_hash?: string;
  };
  scripts: Record<string, import("../../types/script").NarrationScript>;
}

export const loadProjectFull = (id: string) =>
  invoke<LoadedProject>("load_project_full", { id });

export const listProjectFrames = (projectId: string) =>
  invoke<{ index: number; path: string }[]>("list_project_frames", { projectId });

// System
export const getHomeDir = () => invoke<string>("get_home_dir");
export const setMenuContext = (hasProject: boolean) =>
  invoke<void>("set_menu_context", { hasProject });

// Screen recording
export const recordScreenNative = (projectId: string) => invoke<string>("record_screen_native", { projectId });
export const startScreenRecording = (projectId: string) => invoke<void>("start_screen_recording", { projectId });
export const pauseRecording = () => invoke<void>("pause_recording");
export const resumeRecording = () => invoke<void>("resume_recording");
export const stopScreenRecording = () => invoke<string>("stop_screen_recording");
export const getRecordingsDirectory = () => invoke<string>("get_recordings_directory");

// Video editing
export interface VideoEditPlan {
  clips: {
    start_seconds: number; end_seconds: number; speed: number; fps_override: number | null;
    /** "Time-lapse" mode for sped-up clips: silences the clip's audio instead
     *  of atempo-compressing it. Video frames are unaffected. Optional because
     *  the Rust `EditClip` defaults it to false via #[serde(default)]. */
    skip_frames?: boolean;
    clip_type?: string; freeze_source_time?: number; freeze_duration?: number;
    /** Output-timeline duration for image clips. */
    image_duration?: number;
    /** Per-clip source file for multi-source projects. When absent the
     *  Rust render falls back to the `inputPath` arg of applyVideoEdits. */
    input_path?: string;
    zoom_pan?: { startRegion: { x: number; y: number; width: number; height: number }; endRegion: { x: number; y: number; width: number; height: number }; easing: string } | null;
  }[];
  // Rust overlay effect structs use #[serde(rename_all = "camelCase")] so
  // these keys MUST be camelCase. Keep in sync with OverlayEffect in video_edit.rs.
  effects?: {
    type: string;
    startTime: number;
    endTime: number;
    transitionIn?: number;
    transitionOut?: number;
    reverse?: boolean;
    spotlight?: { x: number; y: number; radius: number; dimOpacity: number };
    blur?: { x: number; y: number; width: number; height: number; radius: number; invert?: boolean };
    text?: { content: string; x: number; y: number; fontSize: number; color: string; fontFamily?: string; bold?: boolean; italic?: boolean; underline?: boolean; background?: string; align?: string; opacity?: number };
    fade?: { color: string; opacity: number };
    zoomPan?: { startRegion: { x: number; y: number; width: number; height: number }; endRegion: { x: number; y: number; width: number; height: number }; easing: string };
  }[];
}
/** How much encode time to spend. `final` is the deliverable and the default;
 *  `preview` (720p) and `draft` (480p) trade quality for speed when the user is
 *  only checking that an edit looks right. */
export type RenderQuality = "final" | "preview" | "draft";

export const applyVideoEdits = (
  inputPath: string,
  outputPath: string,
  edits: VideoEditPlan,
  channel: Channel<import("../../types/processing").ProgressEvent>,
  quality?: RenderQuality,
) =>
  invoke<string>("apply_video_edits", { inputPath, outputPath, edits, channel, quality });

/** Rough forecast of what a generation will cost, for the pre-flight panel.
 *  No API calls — safe to call on every settings change. */
export interface GenerationEstimate {
  frame_count: number;
  image_count: number;
  request_count: number;
  input_tokens: number;
  output_tokens: number;
  includes_survey: boolean;
  reuses_cached_frames: boolean;
  /** Ready-made one-line summary, formatted (and tested) backend-side. */
  summary: string;
}

export const estimateGenerationCost = (
  videoPath: string,
  projectId: string,
  frameConfig: { density: string; scene_threshold: number; max_frames: number },
  options?: {
    useContactSheets?: boolean;
    useModelFrameSelection?: boolean;
    strictMode?: boolean;
  },
) =>
  invoke<GenerationEstimate>("estimate_generation_cost", {
    videoPath,
    projectId,
    frameConfig,
    useContactSheets: options?.useContactSheets ?? false,
    useModelFrameSelection: options?.useModelFrameSelection ?? false,
    strictMode: options?.strictMode ?? false,
  });

/** Cancel an in-flight video operation (edit render / audio merge / subtitle burn). */
export const cancelVideoOperation = () => invoke<void>("cancel_video_operation");

export const extractEditThumbnails = (videoPath: string, outputDir: string, count: number) =>
  invoke<string[]>("extract_edit_thumbnails", { videoPath, outputDir, count });

export const extractSingleFrame = (videoPath: string, timestamp: number, outputPath: string) =>
  invoke<string>("extract_single_frame", { videoPath, timestamp, outputPath });

export const saveScript = (projectId: string, language: string, script: import("../../types/script").NarrationScript) =>
  invoke<string>("save_script", { projectId, language, script });

export interface MergeOutcome {
  output_path: string;
  fell_back_to_narration_only: boolean;
}

export const mergeAudioVideo = (
  videoPath: string,
  audioPath: string,
  outputPath: string,
  replaceAudio: boolean,
  channel: Channel<import("../../types/processing").ProgressEvent>,
  duckDb?: number,
) =>
  invoke<MergeOutcome>("merge_audio_video", { videoPath, audioPath, outputPath, replaceAudio, channel, duckDb });

export const openFolder = (path: string) => invoke<void>("open_folder", { path });

// ElevenLabs
export interface ElevenLabsConfig {
  api_key: string;
  voice_id: string;
  model_id: string;
  stability: number;
  similarity_boost: number;
  style: number;
  speed: number;
}

export interface ElevenLabsVoice {
  voice_id: string;
  name: string;
  category: string;
}

export interface TtsResult {
  segment_index: number;
  file_path: string;
  success: boolean;
  error?: string;
}

export const getElevenLabsConfig = () => invoke<ElevenLabsConfig | null>("get_elevenlabs_config");
export const saveElevenLabsConfig = (config: ElevenLabsConfig) => invoke<void>("save_elevenlabs_config", { config });
export const listElevenLabsVoices = (apiKey: string) => invoke<ElevenLabsVoice[]>("list_elevenlabs_voices", { apiKey });
export const validateElevenLabsKey = (apiKey: string) => invoke<boolean>("validate_elevenlabs_key", { apiKey });
export const generateTts = (segments: import("../../types/script").Segment[], outputDir: string, compact: boolean, channel: Channel<import("../../types/processing").ProgressEvent>, ttsProvider?: string) =>
  invoke<TtsResult[]>("generate_tts", { segments, outputDir, compact, channel, ttsProvider: ttsProvider || "elevenlabs" });

// Azure TTS
export interface AzureTtsConfig {
  api_key: string;
  region: string;
  voice_name: string;
  speaking_style: string;
  speed: number;
}

export interface AzureTtsVoice {
  short_name: string;
  display_name: string;
  locale: string;
  gender: string;
}

export const getAzureTtsConfig = () => invoke<AzureTtsConfig | null>("get_azure_tts_config");
export const saveAzureTtsConfig = (config: AzureTtsConfig) => invoke<void>("save_azure_tts_config", { config });
export const listAzureTtsVoices = (apiKey: string, region: string) => invoke<AzureTtsVoice[]>("list_azure_tts_voices", { apiKey, region });
export const validateAzureTtsKey = (apiKey: string, region: string) => invoke<boolean>("validate_azure_tts_key", { apiKey, region });

// Built-in TTS
export interface BuiltinVoice {
  id: string;
  name: string;
  locale: string;
}

export const listBuiltinVoices = () =>
  invoke<BuiltinVoice[]>("list_builtin_voices");

// TTS provider preference
export const getTtsProvider = () => invoke<string | null>("get_tts_provider");
export const saveTtsProvider = (provider: string) => invoke<void>("save_tts_provider", { provider });

// Export
export const exportScript = (options: ExportOptions) =>
  invoke<ExportResult[]>("export_script", { options });

export interface SubtitleStyle {
  font_size: number;
  color: string;
  outline_color: string;
  outline: number;
  position: string;
  /** Optional text transform applied to the SRT before libass renders it.
   *  Currently only "uppercase" is recognized; other values are passthrough. */
  text_transform?: string | null;
  /** Optional re-wrap so each cue renders with at most N words per line. */
  max_words_per_line?: number | null;
}

export const burnSubtitles = (
  videoPath: string,
  script: import("../../types/script").NarrationScript,
  outputPath: string,
  channel: Channel<import("../../types/processing").ProgressEvent>,
  style?: SubtitleStyle,
  audioDir?: string,
  cleanupIntermediate?: string,
) =>
  invoke<string>("burn_subtitles", { videoPath, script, outputPath, channel, style, audioDir, cleanupIntermediate });

// Styles
export const listStyles = () =>
  invoke<NarrationStyleDef[]>("list_styles");

// Telemetry
export const getTelemetryEnabled = () =>
  invoke<boolean>("get_telemetry_enabled");

export const setTelemetryEnabled = (enabled: boolean) =>
  invoke<void>("set_telemetry_enabled", { enabled });
