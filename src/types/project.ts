export interface VideoFile {
  path: string;
  name: string;
  size: number;
  duration: number;
  resolution: { width: number; height: number };
  codec: string;
  fps: number;
}

export interface ContextDocument {
  id: string;
  path: string;
  name: string;
  size: number;
  type: "md" | "txt" | "pdf";
  tokenCount?: number;
}

export interface VideoMetadata {
  path: string;
  duration_seconds: number;
  width: number;
  height: number;
  codec: string;
  fps: number;
  file_size: number;
}
