import type { NarrationScript } from "./script";

export type ExportFormat = "json" | "srt" | "vtt" | "txt" | "md" | "ssml";

export interface ExportOptions {
  formats: ExportFormat[];
  languages: string[];
  output_directory: string;
  scripts: Record<string, NarrationScript>;
  basename?: string;
}

export interface ExportResult {
  format: string;
  language: string;
  file_path: string;
  success: boolean;
  error?: string;
}
