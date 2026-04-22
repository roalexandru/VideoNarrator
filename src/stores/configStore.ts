import { create } from "zustand";
import type {
  NarrationStyleId,
  LanguageCode,
  FrameDensity,
  AiProvider,
  ModelId,
  TtsProvider,
} from "../types/config";

interface ConfigStore {
  style: NarrationStyleId;
  languages: LanguageCode[];
  primaryLanguage: LanguageCode;
  frameDensity: FrameDensity;
  // Scene threshold — reserved for future use, not exposed in UI
  sceneThreshold: number;
  maxFrames: number;
  customPrompt: string;
  aiProvider: AiProvider;
  model: ModelId;
  temperature: number;
  ttsProvider: TtsProvider;
  /** Run an extra critique+refine pass after the initial narration. Catches
   *  segments whose text doesn't match what's visible at that timestamp, at
   *  the cost of one multimodal API call plus up to five refine calls per
   *  iteration. Off by default. */
  strictMode: boolean;

  setStyle: (style: NarrationStyleId) => void;
  toggleLanguage: (lang: LanguageCode) => void;
  setPrimaryLanguage: (lang: LanguageCode) => void;
  setFrameDensity: (density: FrameDensity) => void;
  // Scene threshold — reserved for future use, not exposed in UI
  setSceneThreshold: (threshold: number) => void;
  setMaxFrames: (max: number) => void;
  setCustomPrompt: (prompt: string) => void;
  setAiProvider: (provider: AiProvider) => void;
  setModel: (model: ModelId) => void;
  setTemperature: (temp: number) => void;
  setTtsProvider: (provider: TtsProvider) => void;
  setStrictMode: (enabled: boolean) => void;
  reset: () => void;
}

export const useConfigStore = create<ConfigStore>((set) => ({
  style: "product_demo",
  languages: ["en"],
  primaryLanguage: "en",
  frameDensity: "medium",
  sceneThreshold: 0.3,
  maxFrames: 30,
  customPrompt: "",
  aiProvider: "claude",
  model: "claude-sonnet-4-20250514",
  temperature: 0.7,
  ttsProvider: "elevenlabs",
  strictMode: false,

  setStyle: (style) => set({ style }),

  toggleLanguage: (lang) =>
    set((state) => {
      const has = state.languages.includes(lang);
      const languages = has
        ? state.languages.filter((l) => l !== lang)
        : [...state.languages, lang];
      // If primary language was removed, set first available
      const primaryLanguage = languages.includes(state.primaryLanguage)
        ? state.primaryLanguage
        : languages[0] || "en";
      return { languages, primaryLanguage };
    }),

  setPrimaryLanguage: (lang) => set({ primaryLanguage: lang }),
  setFrameDensity: (density) => set({ frameDensity: density }),
  setSceneThreshold: (threshold) => set({ sceneThreshold: threshold }),
  setMaxFrames: (max) => set({ maxFrames: max }),
  setCustomPrompt: (prompt) => set({ customPrompt: prompt }),
  setAiProvider: (provider) => set({ aiProvider: provider }),
  setModel: (model) => set({ model }),
  setTemperature: (temp) => set({ temperature: temp }),
  setTtsProvider: (provider) => set({ ttsProvider: provider }),
  setStrictMode: (enabled) => set({ strictMode: enabled }),
  reset: () =>
    set({
      style: "product_demo",
      languages: ["en"],
      primaryLanguage: "en",
      frameDensity: "medium",
      sceneThreshold: 0.3,
      maxFrames: 30,
      customPrompt: "",
      aiProvider: "claude",
      model: "claude-sonnet-4-20250514",
      temperature: 0.7,
      ttsProvider: "elevenlabs",
      strictMode: false,
    }),
}));
