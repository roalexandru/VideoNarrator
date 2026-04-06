import { describe, it, expect, beforeEach } from "vitest";
import { useProcessingStore } from "./processingStore";

describe("processingStore", () => {
  beforeEach(() => {
    useProcessingStore.getState().reset();
  });

  it("has correct initial state", () => {
    const state = useProcessingStore.getState();
    expect(state.phase).toBe("idle");
    expect(state.progress).toBe(0);
    expect(state.frames).toEqual([]);
    expect(state.streamingSegments).toEqual([]);
    expect(state.error).toBeNull();
  });

  it("sets phase", () => {
    useProcessingStore.getState().setPhase("extracting_frames");
    expect(useProcessingStore.getState().phase).toBe("extracting_frames");
  });

  it("sets progress", () => {
    useProcessingStore.getState().setProgress(50);
    expect(useProcessingStore.getState().progress).toBe(50);
  });

  it("appends frames", () => {
    useProcessingStore.getState().appendFrame({ index: 0, path: "/tmp/frame0.png", timestamp_seconds: 0, width: 1920, height: 1080 });
    useProcessingStore.getState().appendFrame({ index: 1, path: "/tmp/frame1.png", timestamp_seconds: 5, width: 1920, height: 1080 });
    expect(useProcessingStore.getState().frames).toHaveLength(2);
    expect(useProcessingStore.getState().frames[1].path).toBe("/tmp/frame1.png");
  });

  it("appends segments", () => {
    useProcessingStore.getState().appendSegment({
      index: 0, start_seconds: 0, end_seconds: 5,
      text: "Hello", visual_description: "Intro",
      emphasis: [], pace: "medium", pause_after_ms: 0, frame_refs: [0],
    });
    expect(useProcessingStore.getState().streamingSegments).toHaveLength(1);
    expect(useProcessingStore.getState().streamingSegments[0].text).toBe("Hello");
  });

  it("sets and clears error", () => {
    useProcessingStore.getState().setError("Something went wrong");
    expect(useProcessingStore.getState().error).toBe("Something went wrong");

    useProcessingStore.getState().setError(null);
    expect(useProcessingStore.getState().error).toBeNull();
  });

  it("resets to initial state", () => {
    useProcessingStore.getState().setPhase("generating_narration");
    useProcessingStore.getState().setProgress(75);
    useProcessingStore.getState().setError("err");
    useProcessingStore.getState().appendFrame({ index: 0, path: "/tmp/f.png", timestamp_seconds: 0, width: 1920, height: 1080 });
    useProcessingStore.getState().reset();

    const state = useProcessingStore.getState();
    expect(state.phase).toBe("idle");
    expect(state.progress).toBe(0);
    expect(state.frames).toEqual([]);
    expect(state.error).toBeNull();
  });
});
