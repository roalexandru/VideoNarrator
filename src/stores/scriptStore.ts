import { create } from "zustand";
import type { NarrationScript, Segment } from "../types/script";

interface ScriptStore {
  scripts: Record<string, NarrationScript>;
  activeLanguage: string;
  activeSegmentIndex: number | null;

  setScript: (lang: string, script: NarrationScript) => void;
  setActiveLanguage: (lang: string) => void;
  setActiveSegment: (index: number | null) => void;
  updateSegmentText: (lang: string, index: number, text: string) => void;
  updateSegmentTiming: (
    lang: string,
    index: number,
    start: number,
    end: number
  ) => void;
  deleteSegment: (lang: string, index: number) => void;
  splitSegment: (lang: string, index: number, splitAtSeconds: number) => void;
  mergeSegments: (lang: string, startIndex: number, endIndex: number) => void;
  reset: () => void;
}

export const useScriptStore = create<ScriptStore>((set) => ({
  scripts: {},
  activeLanguage: "en",
  activeSegmentIndex: null,

  setScript: (lang, script) =>
    set((state) => ({
      scripts: { ...state.scripts, [lang]: script },
    })),

  setActiveLanguage: (lang) => set({ activeLanguage: lang }),
  setActiveSegment: (index) => set({ activeSegmentIndex: index }),

  updateSegmentText: (lang, index, text) =>
    set((state) => {
      const script = state.scripts[lang];
      if (!script) return state;
      const segments = [...script.segments];
      segments[index] = { ...segments[index], text };
      return {
        scripts: {
          ...state.scripts,
          [lang]: { ...script, segments },
        },
      };
    }),

  updateSegmentTiming: (lang, index, start, end) =>
    set((state) => {
      const script = state.scripts[lang];
      if (!script) return state;
      const segments = [...script.segments];
      segments[index] = {
        ...segments[index],
        start_seconds: start,
        end_seconds: end,
      };
      return {
        scripts: {
          ...state.scripts,
          [lang]: { ...script, segments },
        },
      };
    }),

  deleteSegment: (lang, index) =>
    set((state) => {
      const script = state.scripts[lang];
      if (!script) return state;
      const segments = script.segments
        .filter((_, i) => i !== index)
        .map((s, i) => ({ ...s, index: i }));
      return {
        scripts: {
          ...state.scripts,
          [lang]: { ...script, segments },
        },
      };
    }),

  splitSegment: (lang, index, splitAtSeconds) =>
    set((state) => {
      const script = state.scripts[lang];
      if (!script) return state;
      const seg = script.segments[index];
      if (!seg || splitAtSeconds <= seg.start_seconds || splitAtSeconds >= seg.end_seconds)
        return state;

      // Split text at nearest word boundary to midpoint
      const midpoint = Math.floor(seg.text.length / 2);
      let splitPos = midpoint;
      // Search for nearest space around midpoint
      for (let offset = 0; offset < midpoint; offset++) {
        if (seg.text[midpoint + offset] === ' ') { splitPos = midpoint + offset; break; }
        if (midpoint - offset >= 0 && seg.text[midpoint - offset] === ' ') { splitPos = midpoint - offset; break; }
      }
      const firstText = seg.text.slice(0, splitPos).trim();
      const secondText = seg.text.slice(splitPos).trim();

      const first: Segment = {
        ...seg,
        end_seconds: splitAtSeconds,
        text: firstText,
      };
      const second: Segment = {
        ...seg,
        index: index + 1,
        start_seconds: splitAtSeconds,
        text: secondText,
      };

      const segments = [
        ...script.segments.slice(0, index),
        first,
        second,
        ...script.segments.slice(index + 1),
      ].map((s, i) => ({ ...s, index: i }));

      return {
        scripts: {
          ...state.scripts,
          [lang]: { ...script, segments },
        },
      };
    }),

  mergeSegments: (lang, startIndex, endIndex) =>
    set((state) => {
      const script = state.scripts[lang];
      if (!script) return state;
      const toMerge = script.segments.slice(startIndex, endIndex + 1);
      if (toMerge.length < 2) return state;

      const merged: Segment = {
        ...toMerge[0],
        end_seconds: toMerge[toMerge.length - 1].end_seconds,
        text: toMerge.map((s) => s.text).join(" "),
        frame_refs: toMerge.flatMap((s) => s.frame_refs),
      };

      const segments = [
        ...script.segments.slice(0, startIndex),
        merged,
        ...script.segments.slice(endIndex + 1),
      ].map((s, i) => ({ ...s, index: i }));

      return {
        scripts: {
          ...state.scripts,
          [lang]: { ...script, segments },
        },
      };
    }),

  reset: () =>
    set({
      scripts: {},
      activeLanguage: "en",
      activeSegmentIndex: null,
    }),
}));
