import { describe, it, expect, beforeEach } from "vitest";
import { setupDefaultMocks, resetAllStores } from "./setup";
import { useProcessingStore } from "../stores/processingStore";
import type { Frame, ProcessingPhase } from "../types/processing";
import type { Segment } from "../types/script";

beforeEach(() => {
  setupDefaultMocks();
  resetAllStores();
});

function makeFrame(index: number): Frame {
  return {
    index,
    timestamp_seconds: index * 5,
    path: `/tmp/frames/frame_${index}.png`,
    width: 1920,
    height: 1080,
  };
}

function makeSegment(index: number): Segment {
  return {
    index,
    start_seconds: index * 10,
    end_seconds: (index + 1) * 10,
    text: `Segment ${index} text`,
    visual_description: `Visual ${index}`,
    emphasis: [],
    pace: "medium",
    pause_after_ms: 0,
    frame_refs: [index],
  };
}

describe("processing state machine", () => {
  it("initial state is idle with no error", () => {
    const state = useProcessingStore.getState();
    expect(state.phase).toBe("idle");
    expect(state.progress).toBe(0);
    expect(state.error).toBeNull();
    expect(state.frames).toEqual([]);
    expect(state.streamingSegments).toEqual([]);
  });

  it("transitions through phases: idle -> extracting_frames -> processing_docs -> generating_narration -> done", () => {
    const store = useProcessingStore.getState();
    const phases: ProcessingPhase[] = [
      "idle",
      "extracting_frames",
      "processing_docs",
      "generating_narration",
      "done",
    ];

    for (const phase of phases) {
      store.setPhase(phase);
      expect(useProcessingStore.getState().phase).toBe(phase);
    }
  });

  it("can transition to error phase", () => {
    const store = useProcessingStore.getState();
    store.setPhase("extracting_frames");
    store.setPhase("error");
    expect(useProcessingStore.getState().phase).toBe("error");
  });

  it("can transition to cancelled phase", () => {
    const store = useProcessingStore.getState();
    store.setPhase("generating_narration");
    store.setPhase("cancelled");
    expect(useProcessingStore.getState().phase).toBe("cancelled");
  });

  it("error state can be set and cleared", () => {
    const store = useProcessingStore.getState();
    store.setError("API rate limit exceeded");
    expect(useProcessingStore.getState().error).toBe("API rate limit exceeded");

    store.setError(null);
    expect(useProcessingStore.getState().error).toBeNull();
  });

  it("error can be set independently of phase", () => {
    const store = useProcessingStore.getState();
    store.setPhase("generating_narration");
    store.setError("Network timeout");

    const state = useProcessingStore.getState();
    expect(state.phase).toBe("generating_narration");
    expect(state.error).toBe("Network timeout");
  });
});

describe("frame accumulation", () => {
  it("appendFrame adds to frames array", () => {
    const store = useProcessingStore.getState();
    store.appendFrame(makeFrame(0));
    store.appendFrame(makeFrame(1));
    store.appendFrame(makeFrame(2));

    const frames = useProcessingStore.getState().frames;
    expect(frames).toHaveLength(3);
    expect(frames[0].index).toBe(0);
    expect(frames[1].index).toBe(1);
    expect(frames[2].index).toBe(2);
  });

  it("frames have correct timestamps", () => {
    const store = useProcessingStore.getState();
    store.appendFrame(makeFrame(0));
    store.appendFrame(makeFrame(3));

    const frames = useProcessingStore.getState().frames;
    expect(frames[0].timestamp_seconds).toBe(0);
    expect(frames[1].timestamp_seconds).toBe(15);
  });

  it("frames have correct paths", () => {
    const store = useProcessingStore.getState();
    store.appendFrame(makeFrame(5));
    expect(useProcessingStore.getState().frames[0].path).toBe("/tmp/frames/frame_5.png");
  });
});

describe("segment accumulation", () => {
  it("appendSegment adds to streamingSegments array", () => {
    const store = useProcessingStore.getState();
    store.appendSegment(makeSegment(0));
    store.appendSegment(makeSegment(1));

    const segments = useProcessingStore.getState().streamingSegments;
    expect(segments).toHaveLength(2);
    expect(segments[0].text).toBe("Segment 0 text");
    expect(segments[1].text).toBe("Segment 1 text");
  });

  it("segments maintain their timing data", () => {
    const store = useProcessingStore.getState();
    store.appendSegment(makeSegment(2));

    const seg = useProcessingStore.getState().streamingSegments[0];
    expect(seg.start_seconds).toBe(20);
    expect(seg.end_seconds).toBe(30);
    expect(seg.frame_refs).toEqual([2]);
  });
});

describe("progress percentage", () => {
  it("progress updates correctly", () => {
    const store = useProcessingStore.getState();
    store.setProgress(0);
    expect(useProcessingStore.getState().progress).toBe(0);

    store.setProgress(50);
    expect(useProcessingStore.getState().progress).toBe(50);

    store.setProgress(100);
    expect(useProcessingStore.getState().progress).toBe(100);
  });

  it("progress can be set to fractional values", () => {
    useProcessingStore.getState().setProgress(33.7);
    expect(useProcessingStore.getState().progress).toBe(33.7);
  });

  it("setProgress is monotonic-forward: smaller values are ignored", () => {
    // Matters because the edit channel (0–35%) closes just as the main
    // channel (0–65%) opens; without the clamp the bar would snap back to 0
    // on the first main tick.
    const store = useProcessingStore.getState();
    store.setProgress(40);
    store.setProgress(20);
    expect(useProcessingStore.getState().progress).toBe(40);
    store.setProgress(45);
    expect(useProcessingStore.getState().progress).toBe(45);
  });

  it("setProgress(0) is an explicit reset — used by the resume flow", () => {
    // The resume flow calls setProgress(0) to re-open the bar from zero.
    // If monotonic clamp treated 0 the same as other smaller values, retry
    // would leave the bar pinned at the pre-failure percent.
    const store = useProcessingStore.getState();
    store.setProgress(75);
    store.setProgress(0);
    expect(useProcessingStore.getState().progress).toBe(0);
  });

  it("setProgress clamps negative values to 0 (defensive)", () => {
    const store = useProcessingStore.getState();
    store.setProgress(50);
    store.setProgress(-5);
    expect(useProcessingStore.getState().progress).toBe(0);
  });
});

describe("status message (live sub-label)", () => {
  it("statusMessage defaults to null", () => {
    expect(useProcessingStore.getState().statusMessage).toBeNull();
  });

  it("setStatusMessage updates the label", () => {
    const store = useProcessingStore.getState();
    store.setStatusMessage("Analyzing batch 2 of 5");
    expect(useProcessingStore.getState().statusMessage).toBe("Analyzing batch 2 of 5");
  });

  it("setStatusMessage(null) clears the label so UI falls back to phase label", () => {
    const store = useProcessingStore.getState();
    store.setStatusMessage("temporary");
    store.setStatusMessage(null);
    expect(useProcessingStore.getState().statusMessage).toBeNull();
  });

  it("reset clears statusMessage alongside other state", () => {
    const store = useProcessingStore.getState();
    store.setStatusMessage("mid-run label");
    store.setProgress(42);
    store.reset();

    const state = useProcessingStore.getState();
    expect(state.statusMessage).toBeNull();
    expect(state.progress).toBe(0);
  });
});

describe("reset returns to initial state", () => {
  it("reset clears everything", () => {
    const store = useProcessingStore.getState();
    store.setPhase("generating_narration");
    store.setProgress(75);
    store.setError("partial error");
    store.appendFrame(makeFrame(0));
    store.appendFrame(makeFrame(1));
    store.appendSegment(makeSegment(0));

    store.reset();

    const state = useProcessingStore.getState();
    expect(state.phase).toBe("idle");
    expect(state.progress).toBe(0);
    expect(state.error).toBeNull();
    expect(state.frames).toEqual([]);
    expect(state.streamingSegments).toEqual([]);
  });
});

describe("error during processing preserves partial results", () => {
  it("error preserves frames already extracted", () => {
    const store = useProcessingStore.getState();
    store.setPhase("extracting_frames");

    // Simulate extracting some frames
    store.appendFrame(makeFrame(0));
    store.appendFrame(makeFrame(1));
    store.appendFrame(makeFrame(2));
    store.setProgress(60);

    // Error occurs
    store.setPhase("error");
    store.setError("ffmpeg crashed");

    const state = useProcessingStore.getState();
    expect(state.phase).toBe("error");
    expect(state.error).toBe("ffmpeg crashed");
    // Frames are preserved
    expect(state.frames).toHaveLength(3);
    expect(state.progress).toBe(60);
  });

  it("error during narration preserves frames and partial segments", () => {
    const store = useProcessingStore.getState();

    // Extract frames first
    store.setPhase("extracting_frames");
    store.appendFrame(makeFrame(0));
    store.appendFrame(makeFrame(1));

    // Move to narration
    store.setPhase("generating_narration");
    store.appendSegment(makeSegment(0));

    // Error occurs mid-generation
    store.setPhase("error");
    store.setError("API quota exceeded");

    const state = useProcessingStore.getState();
    expect(state.frames).toHaveLength(2);
    expect(state.streamingSegments).toHaveLength(1);
    expect(state.streamingSegments[0].text).toBe("Segment 0 text");
  });
});
