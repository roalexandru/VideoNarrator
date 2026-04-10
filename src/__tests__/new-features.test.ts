import { describe, it, expect, beforeEach } from "vitest";
import { setupDefaultMocks, resetAllStores } from "./setup";
import { useScriptStore } from "../stores/scriptStore";
import { useExportStore } from "../stores/exportStore";
import type { NarrationScript, Segment } from "../types/script";
import { cleanPath, fileNameFromPath } from "../lib/formatters";

beforeEach(() => {
  setupDefaultMocks();
  resetAllStores();
});

function makeSeg(overrides: Partial<Segment> = {}): Segment {
  return {
    index: 0, start_seconds: 0, end_seconds: 10,
    text: "Hello world this is a test",
    visual_description: "Title screen", emphasis: [],
    pace: "medium", pause_after_ms: 0, frame_refs: [0],
    ...overrides,
  };
}

function makeScript(segments: Segment[]): NarrationScript {
  return {
    title: "Test Script", total_duration_seconds: 60, segments,
    metadata: { style: "product_demo", language: "en", provider: "claude", model: "claude-sonnet-4-20250514", generated_at: "2026-04-03T14:00:00Z" },
  };
}

// ── Feature #8: Undo/Redo ──

describe("Script Undo/Redo", () => {
  it("canUndo returns false initially", () => {
    expect(useScriptStore.getState().canUndo()).toBe(false);
  });

  it("canRedo returns false initially", () => {
    expect(useScriptStore.getState().canRedo()).toBe(false);
  });

  it("undo restores previous text after updateSegmentText", () => {
    const script = makeScript([makeSeg({ text: "Original" })]);
    useScriptStore.getState().setScript("en", script);
    useScriptStore.getState().updateSegmentText("en", 0, "Modified");

    expect(useScriptStore.getState().scripts["en"].segments[0].text).toBe("Modified");
    expect(useScriptStore.getState().canUndo()).toBe(true);

    useScriptStore.getState().undo();
    expect(useScriptStore.getState().scripts["en"].segments[0].text).toBe("Original");
  });

  it("redo re-applies change after undo", () => {
    const script = makeScript([makeSeg({ text: "Original" })]);
    useScriptStore.getState().setScript("en", script);
    useScriptStore.getState().updateSegmentText("en", 0, "Modified");
    useScriptStore.getState().undo();

    expect(useScriptStore.getState().canRedo()).toBe(true);
    useScriptStore.getState().redo();
    expect(useScriptStore.getState().scripts["en"].segments[0].text).toBe("Modified");
  });

  it("undo restores deleted segment", () => {
    const script = makeScript([
      makeSeg({ index: 0, text: "First" }),
      makeSeg({ index: 1, text: "Second", start_seconds: 10, end_seconds: 20 }),
    ]);
    useScriptStore.getState().setScript("en", script);
    useScriptStore.getState().deleteSegment("en", 1);

    expect(useScriptStore.getState().scripts["en"].segments).toHaveLength(1);
    useScriptStore.getState().undo();
    expect(useScriptStore.getState().scripts["en"].segments).toHaveLength(2);
    expect(useScriptStore.getState().scripts["en"].segments[1].text).toBe("Second");
  });

  it("undo restores timing change", () => {
    const script = makeScript([makeSeg({ start_seconds: 0, end_seconds: 10 })]);
    useScriptStore.getState().setScript("en", script);
    useScriptStore.getState().updateSegmentTiming("en", 0, 2, 8);

    expect(useScriptStore.getState().scripts["en"].segments[0].start_seconds).toBe(2);
    useScriptStore.getState().undo();
    expect(useScriptStore.getState().scripts["en"].segments[0].start_seconds).toBe(0);
    expect(useScriptStore.getState().scripts["en"].segments[0].end_seconds).toBe(10);
  });

  it("undo restores split segment", () => {
    const script = makeScript([makeSeg({ text: "Hello beautiful world", start_seconds: 0, end_seconds: 20 })]);
    useScriptStore.getState().setScript("en", script);
    useScriptStore.getState().splitSegment("en", 0, 10);

    expect(useScriptStore.getState().scripts["en"].segments).toHaveLength(2);
    useScriptStore.getState().undo();
    expect(useScriptStore.getState().scripts["en"].segments).toHaveLength(1);
    expect(useScriptStore.getState().scripts["en"].segments[0].text).toBe("Hello beautiful world");
  });

  it("undo restores merged segments", () => {
    const script = makeScript([
      makeSeg({ index: 0, text: "Hello", start_seconds: 0, end_seconds: 10, frame_refs: [0] }),
      makeSeg({ index: 1, text: "world", start_seconds: 10, end_seconds: 20, frame_refs: [1] }),
    ]);
    useScriptStore.getState().setScript("en", script);
    useScriptStore.getState().mergeSegments("en", 0, 1);

    expect(useScriptStore.getState().scripts["en"].segments).toHaveLength(1);
    useScriptStore.getState().undo();
    expect(useScriptStore.getState().scripts["en"].segments).toHaveLength(2);
    expect(useScriptStore.getState().scripts["en"].segments[0].text).toBe("Hello");
    expect(useScriptStore.getState().scripts["en"].segments[1].text).toBe("world");
  });

  it("new action clears redo stack", () => {
    const script = makeScript([makeSeg({ text: "A" })]);
    useScriptStore.getState().setScript("en", script);
    useScriptStore.getState().updateSegmentText("en", 0, "B");
    useScriptStore.getState().undo();

    expect(useScriptStore.getState().canRedo()).toBe(true);
    useScriptStore.getState().updateSegmentText("en", 0, "C");
    expect(useScriptStore.getState().canRedo()).toBe(false);
  });

  it("setScript does NOT push to undo stack", () => {
    useScriptStore.getState().setScript("en", makeScript([makeSeg()]));
    expect(useScriptStore.getState().canUndo()).toBe(false);
  });

  it("reset clears undo and redo stacks", () => {
    const script = makeScript([makeSeg({ text: "A" })]);
    useScriptStore.getState().setScript("en", script);
    useScriptStore.getState().updateSegmentText("en", 0, "B");

    expect(useScriptStore.getState().canUndo()).toBe(true);
    useScriptStore.getState().reset();
    expect(useScriptStore.getState().canUndo()).toBe(false);
    expect(useScriptStore.getState().canRedo()).toBe(false);
  });

  it("undo stack is capped at 30 entries", () => {
    const script = makeScript([makeSeg({ text: "start" })]);
    useScriptStore.getState().setScript("en", script);

    for (let i = 0; i < 35; i++) {
      useScriptStore.getState().updateSegmentText("en", 0, `edit-${i}`);
    }

    // Stack should be capped
    const state = useScriptStore.getState();
    expect(state.undoStack.length).toBeLessThanOrEqual(30);
    expect(state.canUndo()).toBe(true);
  });

  it("multiple undos walk back through history", () => {
    const script = makeScript([makeSeg({ text: "A" })]);
    useScriptStore.getState().setScript("en", script);
    useScriptStore.getState().updateSegmentText("en", 0, "B");
    useScriptStore.getState().updateSegmentText("en", 0, "C");
    useScriptStore.getState().updateSegmentText("en", 0, "D");

    useScriptStore.getState().undo(); // D -> C
    expect(useScriptStore.getState().scripts["en"].segments[0].text).toBe("C");
    useScriptStore.getState().undo(); // C -> B
    expect(useScriptStore.getState().scripts["en"].segments[0].text).toBe("B");
    useScriptStore.getState().undo(); // B -> A
    expect(useScriptStore.getState().scripts["en"].segments[0].text).toBe("A");
    expect(useScriptStore.getState().canUndo()).toBe(false);
  });
});

// ── Feature #4: Segment Timing ──

describe("Segment Timing Update", () => {
  it("updates start and end seconds", () => {
    const script = makeScript([makeSeg({ start_seconds: 0, end_seconds: 10 })]);
    useScriptStore.getState().setScript("en", script);
    useScriptStore.getState().updateSegmentTiming("en", 0, 2, 8);

    const seg = useScriptStore.getState().scripts["en"].segments[0];
    expect(seg.start_seconds).toBe(2);
    expect(seg.end_seconds).toBe(8);
  });

  it("preserves other segment fields", () => {
    const script = makeScript([makeSeg({ text: "Hello", pace: "fast", frame_refs: [1, 2] })]);
    useScriptStore.getState().setScript("en", script);
    useScriptStore.getState().updateSegmentTiming("en", 0, 1, 9);

    const seg = useScriptStore.getState().scripts["en"].segments[0];
    expect(seg.text).toBe("Hello");
    expect(seg.pace).toBe("fast");
    expect(seg.frame_refs).toEqual([1, 2]);
  });

  it("does nothing for non-existent language", () => {
    useScriptStore.getState().updateSegmentTiming("fr", 0, 5, 15);
    expect(useScriptStore.getState().scripts["fr"]).toBeUndefined();
  });
});

// ── Feature #5: Subtitle Style (Export Store) ──

describe("Export Store — Subtitle Style Defaults", () => {
  it("has correct initial subtitle state", () => {
    const state = useExportStore.getState();
    expect(state.subtitleFontSize).toBe(22);
    expect(state.subtitleColor).toBe("#ffffff");
    expect(state.subtitleOutlineColor).toBe("#000000");
    expect(state.subtitleOutline).toBe(2);
    expect(state.subtitlePosition).toBe("bottom");
  });

  it("setSubtitleFontSize updates correctly", () => {
    useExportStore.getState().setSubtitleFontSize(36);
    expect(useExportStore.getState().subtitleFontSize).toBe(36);
  });

  it("setSubtitleColor updates correctly", () => {
    useExportStore.getState().setSubtitleColor("#ffff00");
    expect(useExportStore.getState().subtitleColor).toBe("#ffff00");
  });

  it("setSubtitleOutlineColor updates correctly", () => {
    useExportStore.getState().setSubtitleOutlineColor("#333333");
    expect(useExportStore.getState().subtitleOutlineColor).toBe("#333333");
  });

  it("setSubtitleOutline updates correctly", () => {
    useExportStore.getState().setSubtitleOutline(4);
    expect(useExportStore.getState().subtitleOutline).toBe(4);
  });

  it("setSubtitlePosition updates correctly", () => {
    useExportStore.getState().setSubtitlePosition("top");
    expect(useExportStore.getState().subtitlePosition).toBe("top");
  });

  it("reset restores subtitle defaults", () => {
    useExportStore.getState().setSubtitleFontSize(48);
    useExportStore.getState().setSubtitleColor("#ff0000");
    useExportStore.getState().setSubtitlePosition("top");
    useExportStore.getState().reset();

    const state = useExportStore.getState();
    expect(state.subtitleFontSize).toBe(22);
    expect(state.subtitleColor).toBe("#ffffff");
    expect(state.subtitlePosition).toBe("bottom");
  });
});

// ── Path Helpers (used by drag-and-drop) ──

describe("Path Helpers", () => {
  it("cleanPath strips \\\\?\\ prefix", () => {
    expect(cleanPath("\\\\?\\C:\\Users\\test\\video.mp4")).toBe("C:\\Users\\test\\video.mp4");
  });

  it("cleanPath is a no-op for normal paths", () => {
    expect(cleanPath("C:\\Users\\test\\video.mp4")).toBe("C:\\Users\\test\\video.mp4");
    expect(cleanPath("/Users/test/video.mp4")).toBe("/Users/test/video.mp4");
  });

  it("fileNameFromPath extracts filename from Windows path", () => {
    expect(fileNameFromPath("C:\\Users\\test\\video.mp4")).toBe("video.mp4");
  });

  it("fileNameFromPath extracts filename from Unix path", () => {
    expect(fileNameFromPath("/Users/test/video.mp4")).toBe("video.mp4");
  });

  it("fileNameFromPath extracts filename from path with \\\\?\\ prefix", () => {
    expect(fileNameFromPath("\\\\?\\C:\\Users\\test\\video.mp4")).toBe("video.mp4");
  });

  it("fileNameFromPath returns 'video' for empty path", () => {
    expect(fileNameFromPath("")).toBe("video");
  });
});
