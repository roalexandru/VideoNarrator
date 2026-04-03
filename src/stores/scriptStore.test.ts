import { describe, it, expect, beforeEach } from "vitest";
import { useScriptStore } from "./scriptStore";
import type { NarrationScript, Segment } from "../types/script";

function makeSeg(overrides: Partial<Segment> = {}): Segment {
  return {
    index: 0,
    start_seconds: 0,
    end_seconds: 5,
    text: "Hello world",
    visual_description: "Title screen",
    emphasis: [],
    pace: "medium",
    pause_after_ms: 0,
    frame_refs: [0],
    ...overrides,
  };
}

function makeScript(segments: Segment[]): NarrationScript {
  return {
    title: "Test",
    total_duration_seconds: 30,
    segments,
    metadata: {
      style: "technical",
      language: "en",
      provider: "claude",
      model: "claude-sonnet-4-20250514",
      generated_at: "2026-04-03T14:00:00Z",
    },
  };
}

describe("scriptStore", () => {
  beforeEach(() => {
    useScriptStore.getState().reset();
  });

  it("sets and retrieves a script", () => {
    const script = makeScript([makeSeg()]);
    useScriptStore.getState().setScript("en", script);
    expect(useScriptStore.getState().scripts["en"]).toBeDefined();
    expect(useScriptStore.getState().scripts["en"].segments).toHaveLength(1);
  });

  it("updates segment text", () => {
    const script = makeScript([makeSeg({ text: "original" })]);
    useScriptStore.getState().setScript("en", script);
    useScriptStore.getState().updateSegmentText("en", 0, "modified");
    expect(useScriptStore.getState().scripts["en"].segments[0].text).toBe(
      "modified"
    );
  });

  it("deletes a segment", () => {
    const script = makeScript([
      makeSeg({ index: 0, text: "first" }),
      makeSeg({ index: 1, text: "second", start_seconds: 5, end_seconds: 10 }),
    ]);
    useScriptStore.getState().setScript("en", script);
    useScriptStore.getState().deleteSegment("en", 0);
    const segs = useScriptStore.getState().scripts["en"].segments;
    expect(segs).toHaveLength(1);
    expect(segs[0].text).toBe("second");
    expect(segs[0].index).toBe(0); // re-indexed
  });

  it("splits a segment", () => {
    const script = makeScript([
      makeSeg({ index: 0, start_seconds: 0, end_seconds: 10, text: "Hello world" }),
    ]);
    useScriptStore.getState().setScript("en", script);
    useScriptStore.getState().splitSegment("en", 0, 5);
    const segs = useScriptStore.getState().scripts["en"].segments;
    expect(segs).toHaveLength(2);
    expect(segs[0].end_seconds).toBe(5);
    expect(segs[1].start_seconds).toBe(5);
    expect(segs[1].end_seconds).toBe(10);
  });

  it("merges segments", () => {
    const script = makeScript([
      makeSeg({ index: 0, text: "Hello", start_seconds: 0, end_seconds: 5 }),
      makeSeg({ index: 1, text: "world", start_seconds: 5, end_seconds: 10 }),
      makeSeg({ index: 2, text: "!", start_seconds: 10, end_seconds: 15 }),
    ]);
    useScriptStore.getState().setScript("en", script);
    useScriptStore.getState().mergeSegments("en", 0, 1);
    const segs = useScriptStore.getState().scripts["en"].segments;
    expect(segs).toHaveLength(2);
    expect(segs[0].text).toBe("Hello world");
    expect(segs[0].start_seconds).toBe(0);
    expect(segs[0].end_seconds).toBe(10);
    expect(segs[1].text).toBe("!");
  });

  it("updates segment timing", () => {
    const script = makeScript([
      makeSeg({ start_seconds: 0, end_seconds: 5 }),
    ]);
    useScriptStore.getState().setScript("en", script);
    useScriptStore.getState().updateSegmentTiming("en", 0, 1, 8);
    const seg = useScriptStore.getState().scripts["en"].segments[0];
    expect(seg.start_seconds).toBe(1);
    expect(seg.end_seconds).toBe(8);
  });
});
