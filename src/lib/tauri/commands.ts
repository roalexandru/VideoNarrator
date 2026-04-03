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

export const getProviderStatus = () =>
  invoke<ProviderKeyStatus[]>("get_provider_status");

export const setApiKey = (provider: AiProvider, key: string) =>
  invoke<void>("set_api_key", { provider, key });

export const validateApiKey = (provider: AiProvider, key: string) =>
  invoke<boolean>("validate_api_key_cmd", { provider, key });

// Video
export const probeVideo = (path: string) =>
  invoke<VideoMetadata>("probe_video", { path });

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
    ai_config: { provider: string; model: string; temperature: number };
    custom_prompt: string;
    created_at: string;
    updated_at: string;
  };
  scripts: Record<string, import("../../types/script").NarrationScript>;
}

export const loadProjectFull = (id: string) =>
  invoke<LoadedProject>("load_project_full", { id });

export const listProjectFrames = (projectId: string) =>
  invoke<{ index: number; path: string }[]>("list_project_frames", { projectId });

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
export const generateTts = (segments: import("../../types/script").Segment[], outputDir: string, compact: boolean, channel: Channel<import("../../types/processing").ProgressEvent>) =>
  invoke<TtsResult[]>("generate_tts", { segments, outputDir, compact, channel });

// Export
export const exportScript = (options: ExportOptions) =>
  invoke<ExportResult[]>("export_script", { options });

// Styles
export const listStyles = () =>
  invoke<NarrationStyleDef[]>("list_styles");
