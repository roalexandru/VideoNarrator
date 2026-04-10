import { create } from "zustand";
import type { ExportFormat } from "../types/export";

interface ExportStore {
  selectedFormats: ExportFormat[];
  languageToggles: Record<string, boolean>;
  outputDirectory: string | null;
  basename: string;
  burnSubtitles: boolean;
  replaceAudio: boolean;
  subtitleFontSize: number;
  subtitleColor: string;
  subtitleOutlineColor: string;
  subtitleOutline: number;
  subtitlePosition: "bottom" | "top";

  toggleFormat: (format: ExportFormat) => void;
  toggleLanguageExport: (lang: string) => void;
  setOutputDirectory: (dir: string) => void;
  setBasename: (name: string) => void;
  setBurnSubtitles: (burn: boolean) => void;
  setReplaceAudio: (replace: boolean) => void;
  setSubtitleFontSize: (size: number) => void;
  setSubtitleColor: (color: string) => void;
  setSubtitleOutlineColor: (color: string) => void;
  setSubtitleOutline: (outline: number) => void;
  setSubtitlePosition: (position: "bottom" | "top") => void;
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
  subtitleFontSize: 22,
  subtitleColor: "#ffffff",
  subtitleOutlineColor: "#000000",
  subtitleOutline: 2,
  subtitlePosition: "bottom",

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
  setSubtitleFontSize: (size) => set({ subtitleFontSize: size }),
  setSubtitleColor: (color) => set({ subtitleColor: color }),
  setSubtitleOutlineColor: (color) => set({ subtitleOutlineColor: color }),
  setSubtitleOutline: (outline) => set({ subtitleOutline: outline }),
  setSubtitlePosition: (position) => set({ subtitlePosition: position }),

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
      subtitleFontSize: 22,
      subtitleColor: "#ffffff",
      subtitleOutlineColor: "#000000",
      subtitleOutline: 2,
      subtitlePosition: "bottom",
    }),
}));
