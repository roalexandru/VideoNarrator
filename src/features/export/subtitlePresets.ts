export type SubtitlePreset = "shorts" | "documentary" | "clean" | "custom";

/**
 * Fields a named preset controls. Deliberately excludes `subtitlePosition` —
 * top/bottom placement is a layout concern orthogonal to the visual style, so
 * users can combine any preset with either position without being forced into
 * Custom just to flip placement.
 */
export interface SubtitleStyleFields {
  subtitleFontSize: number;
  subtitleColor: string;
  subtitleOutlineColor: string;
  subtitleOutline: number;
  subtitleTextTransform: "uppercase" | null;
  subtitleMaxWordsPerLine: number | null;
}

type NamedPreset = Exclude<SubtitlePreset, "custom">;

export const SUBTITLE_PRESETS: Record<NamedPreset, SubtitleStyleFields> = {
  shorts: {
    subtitleFontSize: 36,
    subtitleColor: "#ffffff",
    subtitleOutlineColor: "#000000",
    subtitleOutline: 4,
    subtitleTextTransform: "uppercase",
    subtitleMaxWordsPerLine: 2,
  },
  documentary: {
    subtitleFontSize: 22,
    subtitleColor: "#ffffff",
    subtitleOutlineColor: "#000000",
    subtitleOutline: 2,
    subtitleTextTransform: null,
    subtitleMaxWordsPerLine: null,
  },
  clean: {
    subtitleFontSize: 18,
    subtitleColor: "#ffffff",
    subtitleOutlineColor: "#000000",
    subtitleOutline: 1,
    subtitleTextTransform: null,
    subtitleMaxWordsPerLine: null,
  },
};

export const PRESET_LABELS: Record<SubtitlePreset, string> = {
  shorts: "Shorts",
  documentary: "Documentary",
  clean: "Clean",
  custom: "Custom",
};

export const PRESET_DESCRIPTIONS: Record<NamedPreset, string> = {
  shorts: "Large, bold, 2-word uppercase chunks — punchy TikTok/Reels look.",
  documentary: "Balanced white text with a subtle outline — safe for most footage.",
  clean: "Smaller, low-contrast outline — unobtrusive for professional delivery.",
};

/**
 * Detect which preset (if any) the current field values match, so the UI can
 * keep the selector in sync if a user migrates from a saved custom style that
 * happens to align with a preset. Falls back to "custom" the moment any field
 * diverges.
 */
export function detectPreset(fields: SubtitleStyleFields): SubtitlePreset {
  for (const key of ["shorts", "documentary", "clean"] as const) {
    const expected = SUBTITLE_PRESETS[key];
    const allMatch = (Object.keys(expected) as Array<keyof SubtitleStyleFields>).every(
      (k) => fields[k] === expected[k],
    );
    if (allMatch) return key;
  }
  return "custom";
}
