import { create } from "zustand";
import type {
  NarrationStyleId,
  LanguageCode,
  FrameDensity,
  AiProvider,
  ModelId,
  ReasoningEffort,
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
  /** Provider-agnostic thinking depth; mapped per vendor in the Rust client. */
  reasoningEffort: ReasoningEffort;
  ttsProvider: TtsProvider;
  /** Run an extra critique+refine pass after the initial narration. Catches
   *  segments whose text doesn't match what's visible at that timestamp, at
   *  the cost of one multimodal API call plus up to five refine calls per
   *  iteration. Off by default. */
  strictMode: boolean;
  /** Read the text visible on screen (OCR) and give it to the model, so
   *  narration can name the actual command, file or error instead of describing
   *  it generically. On by default; inert on platforms without a recognizer. */
  screenText: boolean;
  /** Let the model pick which moments deserve full-resolution frames, via one
   *  cheap low-resolution survey pass, instead of an even-spaced subsample.
   *  On by default; falls back to even spacing on any failure. */
  modelFrameSelection: boolean;

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
  setReasoningEffort: (effort: ReasoningEffort) => void;
  setTtsProvider: (provider: TtsProvider) => void;
  setStrictMode: (enabled: boolean) => void;
  setScreenText: (enabled: boolean) => void;
  setModelFrameSelection: (enabled: boolean) => void;
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
  model: "claude-sonnet-5",
  temperature: 0.7,
  reasoningEffort: "balanced",
  ttsProvider: "elevenlabs",
  strictMode: false,
  screenText: true,
  modelFrameSelection: true,

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
  setReasoningEffort: (effort) => set({ reasoningEffort: effort }),
  setTtsProvider: (provider) => set({ ttsProvider: provider }),
  setStrictMode: (enabled) => set({ strictMode: enabled }),
  setScreenText: (enabled) => set({ screenText: enabled }),
  setModelFrameSelection: (enabled) => set({ modelFrameSelection: enabled }),
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
      model: "claude-sonnet-5",
      temperature: 0.7,
      reasoningEffort: "balanced",
      ttsProvider: "elevenlabs",
      strictMode: false,
      screenText: true,
      modelFrameSelection: true,
    }),
}));
