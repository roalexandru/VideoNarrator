import type { VideoFile } from "../types/project";

export function canProceedFromStep0(
  videoFile: VideoFile | null,
  title: string
): boolean {
  return videoFile !== null && title.trim().length > 0;
}

export function canProceedFromStep1(languages: string[]): boolean {
  return languages.length > 0;
}

export function canProceedFromStep3(segmentCount: number): boolean {
  return segmentCount > 0;
}
