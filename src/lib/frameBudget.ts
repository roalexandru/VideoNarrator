import type { FrameDensity } from "../types/config";

/** Seconds between sampled frames per density tier.
 *  Must match `FrameDensity::interval_seconds` in src-tauri/src/models.rs. */
export const DENSITY_INTERVAL: Record<FrameDensity, number> = {
  light: 10,
  medium: 5,
  heavy: 2,
};

/** Recommended `max_frames` for an edited video of `duration` seconds at
 *  `density`. Floor of 30 preserves short-video behavior; 300 caps pathological
 *  requests (roughly 10 minutes @ heavy or 50 minutes @ light). */
export function recommendedMaxFrames(durationSeconds: number, density: FrameDensity): number {
  if (durationSeconds <= 0) return 30;
  const raw = Math.ceil(durationSeconds / DENSITY_INTERVAL[density]);
  return Math.min(300, Math.max(30, raw));
}

/** Matches `is_openai_reasoning_model` in src-tauri/src/ai_client.rs.
 *  These models reject any user-set `temperature`. */
export function isReasoningModel(provider: string, model: string): boolean {
  return (
    provider === "openai" &&
    (model.startsWith("o1") ||
      model.startsWith("o3") ||
      model.startsWith("o4") ||
      model.startsWith("gpt-5"))
  );
}
