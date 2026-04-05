import { create } from "zustand";
import type { ExportFormat } from "../types/export";

interface ExportStore {
  selectedFormats: ExportFormat[];
  languageToggles: Record<string, boolean>;
  outputDirectory: string | null;
  basename: string;
  burnSubtitles: boolean;
  replaceAudio: boolean;

  toggleFormat: (format: ExportFormat) => void;
  toggleLanguageExport: (lang: string) => void;
  setOutputDirectory: (dir: string) => void;
  setBasename: (name: string) => void;
  setBurnSubtitles: (burn: boolean) => void;
  setReplaceAudio: (replace: boolean) => void;
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
    }),
}));
