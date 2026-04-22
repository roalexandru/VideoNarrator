import { create } from "zustand";
import type { ExportFormat } from "../types/export";
import { SUBTITLE_PRESETS, type SubtitlePreset } from "../features/export/subtitlePresets";

interface ExportStore {
  selectedFormats: ExportFormat[];
  languageToggles: Record<string, boolean>;
  outputDirectory: string | null;
  basename: string;
  burnSubtitles: boolean;
  replaceAudio: boolean;
  subtitlePreset: SubtitlePreset;
  subtitleFontSize: number;
  subtitleColor: string;
  subtitleOutlineColor: string;
  subtitleOutline: number;
  subtitlePosition: "bottom" | "top";
  /** "uppercase" transforms cue text to ALL CAPS before libass renders it.
   *  null keeps the script's original casing. */
  subtitleTextTransform: "uppercase" | null;
  /** Re-wraps each cue to at most N words per line. null keeps the SRT's
   *  original line breaks. */
  subtitleMaxWordsPerLine: number | null;
  /** Amount (dB) to duck the original audio whenever narration is audible.
   *  Only used when replaceAudio is false. Range: -20..0. */
  duckDb: number;

  toggleFormat: (format: ExportFormat) => void;
  toggleLanguageExport: (lang: string) => void;
  setOutputDirectory: (dir: string) => void;
  setBasename: (name: string) => void;
  setBurnSubtitles: (burn: boolean) => void;
  setReplaceAudio: (replace: boolean) => void;
  setSubtitlePreset: (preset: SubtitlePreset) => void;
  setSubtitleFontSize: (size: number) => void;
  setSubtitleColor: (color: string) => void;
  setSubtitleOutlineColor: (color: string) => void;
  setSubtitleOutline: (outline: number) => void;
  setSubtitlePosition: (position: "bottom" | "top") => void;
  setSubtitleTextTransform: (t: "uppercase" | null) => void;
  setSubtitleMaxWordsPerLine: (n: number | null) => void;
  setDuckDb: (db: number) => void;
  initLanguages: (languages: string[]) => void;
  initFromTitle: (title: string, homeDir: string) => void;
  reset: () => void;
}

/** Convert a project title to a filesystem-safe slug */
export function slugify(title: string): string {
  return title
    .toLowerCase()
    .replace(/[^a-z0-9\s-]/g, "")
    .trim()
    .replace(/\s+/g, "-")
    .replace(/-+/g, "-")
    || "untitled";
}

export const useExportStore = create<ExportStore>((set) => ({
  selectedFormats: ["json", "srt"],
  languageToggles: { en: true },
  outputDirectory: null,
  basename: "untitled",
  burnSubtitles: false,
  replaceAudio: true,
  // Default matches the "documentary" preset (22pt, 2px outline, bottom) —
  // same look as before the preset system shipped.
  subtitlePreset: "documentary",
  subtitleFontSize: 22,
  subtitleColor: "#ffffff",
  subtitleOutlineColor: "#000000",
  subtitleOutline: 2,
  subtitlePosition: "bottom",
  subtitleTextTransform: null,
  subtitleMaxWordsPerLine: null,
  duckDb: -8,

  toggleFormat: (format) =>
    set((state) => {
      const has = state.selectedFormats.includes(format);
      return {
        selectedFormats: has
          ? state.selectedFormats.filter((f) => f !== format)
          : [...state.selectedFormats, format],
      };
    }),

  toggleLanguageExport: (lang) =>
    set((state) => ({
      languageToggles: {
        ...state.languageToggles,
        [lang]: !state.languageToggles[lang],
      },
    })),

  setOutputDirectory: (dir) => set({ outputDirectory: dir }),
  setBasename: (name) => set({ basename: name }),
  setBurnSubtitles: (burn) => set({ burnSubtitles: burn }),
  setReplaceAudio: (replace) => set({ replaceAudio: replace }),
  // Selecting a named preset snaps every style field to the preset's values.
  // Selecting "custom" leaves fields alone so users can tweak from wherever
  // they are without losing their manual choices.
  setSubtitlePreset: (preset) =>
    set(() => {
      if (preset === "custom") return { subtitlePreset: preset };
      return { subtitlePreset: preset, ...SUBTITLE_PRESETS[preset] };
    }),
  // Any direct field edit forces the preset to "custom" so the selector
  // no longer implies the fields are in their preset state.
  setSubtitleFontSize: (size) => set({ subtitleFontSize: size, subtitlePreset: "custom" }),
  setSubtitleColor: (color) => set({ subtitleColor: color, subtitlePreset: "custom" }),
  setSubtitleOutlineColor: (color) => set({ subtitleOutlineColor: color, subtitlePreset: "custom" }),
  setSubtitleOutline: (outline) => set({ subtitleOutline: outline, subtitlePreset: "custom" }),
  // Position is orthogonal to a preset's visual style — flipping top/bottom
  // shouldn't force the user into Custom.
  setSubtitlePosition: (position) => set({ subtitlePosition: position }),
  setSubtitleTextTransform: (t) => set({ subtitleTextTransform: t, subtitlePreset: "custom" }),
  setSubtitleMaxWordsPerLine: (n) => set({ subtitleMaxWordsPerLine: n, subtitlePreset: "custom" }),
  setDuckDb: (db) => set({ duckDb: Math.max(-20, Math.min(0, db)) }),

  initLanguages: (languages) =>
    set({
      languageToggles: Object.fromEntries(languages.map((l) => [l, true])),
    }),

  initFromTitle: (title, homeDir) =>
    set((state) => {
      const slug = slugify(title);
      return {
        basename: state.basename === "untitled" || !state.outputDirectory ? slug : state.basename,
        outputDirectory: state.outputDirectory || `${homeDir}/Documents/Narrator/${slug}`,
      };
    }),

  reset: () =>
    set({
      selectedFormats: ["json", "srt"],
      languageToggles: { en: true },
      outputDirectory: null,
      basename: "untitled",
      burnSubtitles: false,
      replaceAudio: true,
      subtitlePreset: "documentary",
      subtitleFontSize: 22,
      subtitleColor: "#ffffff",
      subtitleOutlineColor: "#000000",
      subtitleOutline: 2,
      subtitlePosition: "bottom",
      subtitleTextTransform: null,
      subtitleMaxWordsPerLine: null,
      duckDb: -8,
    }),
}));
