import { create } from "zustand";
import type { ExportFormat } from "../types/export";

interface ExportStore {
  selectedFormats: ExportFormat[];
  languageToggles: Record<string, boolean>;
  outputDirectory: string | null;

  toggleFormat: (format: ExportFormat) => void;
  toggleLanguageExport: (lang: string) => void;
  setOutputDirectory: (dir: string) => void;
  initLanguages: (languages: string[]) => void;
  reset: () => void;
}

export const useExportStore = create<ExportStore>((set) => ({
  selectedFormats: ["json", "srt"],
  languageToggles: { en: true },
  outputDirectory: null,

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

  initLanguages: (languages) =>
    set({
      languageToggles: Object.fromEntries(languages.map((l) => [l, true])),
    }),

  reset: () =>
    set({
      selectedFormats: ["json", "srt"],
      languageToggles: { en: true },
      outputDirectory: null,
    }),
}));
