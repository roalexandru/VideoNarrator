import { describe, it, expect, beforeEach } from "vitest";
import { setupDefaultMocks, resetAllStores } from "./setup";
import { useEditStore } from "../stores/editStore";

beforeEach(() => {
  setupDefaultMocks();
  resetAllStores();
});

describe("initFromVideo", () => {
  it("creates a single full-length clip", () => {
    useEditStore.getState().initFromVideo(120);
    const { clips, sourceDuration } = useEditStore.getState();
    expect(clips).toHaveLength(1);
    expect(clips[0].sourceStart).toBe(0);
    expect(clips[0].sourceEnd).toBe(120);
    expect(clips[0].speed).toBe(1.0);
    expect(clips[0].skipFrames).toBe(false);
    expect(clips[0].fpsOverride).toBeNull();
    expect(sourceDuration).toBe(120);
  });

  it("clears undo/redo stacks on init", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().splitAt(30);
    expect(useEditStore.getState().canUndo()).toBe(true);

    useEditStore.getState().initFromVideo(90);
    expect(useEditStore.getState().canUndo()).toBe(false);
    expect(useEditStore.getState().canRedo()).toBe(false);
  });
});

describe("splitAt edge cases", () => {
  it("splitAt at the very start (0.1s) creates two clips", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().splitAt(0.1);

    const { clips } = useEditStore.getState();
    expect(clips).toHaveLength(2);
    expect(clips[0].sourceStart).toBe(0);
    expect(clips[0].sourceEnd).toBeCloseTo(0.1, 5);
    expect(clips[1].sourceStart).toBeCloseTo(0.1, 5);
    expect(clips[1].sourceEnd).toBe(60);
  });

  it("splitAt at 0 does nothing (boundary)", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().splitAt(0);
    expect(useEditStore.getState().clips).toHaveLength(1);
  });

  it("splitAt at the very end does nothing (boundary)", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().splitAt(60);
    expect(useEditStore.getState().clips).toHaveLength(1);
  });

  it("splitAt near the end (59.9s) creates two clips", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().splitAt(59.9);

    const { clips } = useEditStore.getState();
    expect(clips).toHaveLength(2);
    expect(clips[0].sourceEnd).toBeCloseTo(59.9, 5);
    expect(clips[1].sourceStart).toBeCloseTo(59.9, 5);
    expect(clips[1].sourceEnd).toBe(60);
  });

  it("splitAt preserves speed from parent clip", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().setClipSpeed(0, 1.5);
    // Output duration = 60/1.5 = 40. Split at output 20 -> source offset = 20 * 1.5 = 30
    useEditStore.getState().splitAt(20);

    const { clips } = useEditStore.getState();
    expect(clips).toHaveLength(2);
    expect(clips[0].speed).toBe(1.5);
    expect(clips[1].speed).toBe(1.5);
    expect(clips[0].sourceEnd).toBeCloseTo(30, 5);
    expect(clips[1].sourceStart).toBeCloseTo(30, 5);
  });
});

describe("deleteClip", () => {
  it("removes the correct clip", () => {
    useEditStore.getState().initFromVideo(90);
    useEditStore.getState().splitAt(30);
    useEditStore.getState().splitAt(60);
    expect(useEditStore.getState().clips).toHaveLength(3);

    const clipIds = useEditStore.getState().clips.map((c) => c.id);

    useEditStore.getState().deleteClip(1);
    const remaining = useEditStore.getState().clips;
    expect(remaining).toHaveLength(2);
    expect(remaining[0].id).toBe(clipIds[0]);
    expect(remaining[1].id).toBe(clipIds[2]);
  });

  it("cannot delete the last remaining clip", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().deleteClip(0);
    expect(useEditStore.getState().clips).toHaveLength(1);
  });

  it("selectedClipIndex adjusts when deleting at end", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().splitAt(30);
    useEditStore.getState().selectClip(1);

    useEditStore.getState().deleteClip(1);
    expect(useEditStore.getState().selectedClipIndex).toBe(0);
  });
});

describe("moveClip", () => {
  it("reorders clips correctly", () => {
    useEditStore.getState().initFromVideo(90);
    useEditStore.getState().splitAt(30);
    useEditStore.getState().splitAt(60);

    const ids = useEditStore.getState().clips.map((c) => c.id);

    // Move last to first position
    useEditStore.getState().moveClip(2, 0);
    const reordered = useEditStore.getState().clips;
    expect(reordered[0].id).toBe(ids[2]);
    expect(reordered[1].id).toBe(ids[0]);
    expect(reordered[2].id).toBe(ids[1]);
    expect(useEditStore.getState().selectedClipIndex).toBe(0);
  });

  it("moveClip with same from/to does nothing", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().splitAt(30);
    const before = useEditStore.getState().clips.map((c) => c.id);

    useEditStore.getState().moveClip(1, 1);
    const after = useEditStore.getState().clips.map((c) => c.id);
    expect(after).toEqual(before);
  });
});

describe("setClipSpeed", () => {
  it("updates speed and recalculates duration", () => {
    useEditStore.getState().initFromVideo(100);
    expect(useEditStore.getState().getOutputDuration()).toBe(100);

    useEditStore.getState().setClipSpeed(0, 2.0);
    expect(useEditStore.getState().clips[0].speed).toBe(2.0);
    expect(useEditStore.getState().getOutputDuration()).toBe(50);
  });

  it("extreme slow speed (0.25x)", () => {
    useEditStore.getState().initFromVideo(10);
    useEditStore.getState().setClipSpeed(0, 0.25);

    expect(useEditStore.getState().clips[0].speed).toBe(0.25);
    expect(useEditStore.getState().getOutputDuration()).toBe(40); // 10 / 0.25
  });

  it("extreme fast speed (4x)", () => {
    useEditStore.getState().initFromVideo(100);
    useEditStore.getState().setClipSpeed(0, 4.0);

    expect(useEditStore.getState().clips[0].speed).toBe(4.0);
    expect(useEditStore.getState().getOutputDuration()).toBe(25); // 100 / 4
  });
});

describe("outputDuration with multiple clips at different speeds", () => {
  it("calculates total with mixed speeds", () => {
    useEditStore.getState().initFromVideo(120);
    useEditStore.getState().splitAt(40); // [0-40] @1x, [40-120] @1x
    useEditStore.getState().splitAt(80); // [0-40], [40-80], [80-120]

    useEditStore.getState().setClipSpeed(0, 2.0);  // 40/2 = 20
    useEditStore.getState().setClipSpeed(1, 0.5);  // 40/0.5 = 80
    useEditStore.getState().setClipSpeed(2, 1.0);  // 40/1 = 40

    expect(useEditStore.getState().getOutputDuration()).toBe(140); // 20+80+40
  });
});

describe("getClipOutputStart", () => {
  it("returns correct offsets for multiple clips", () => {
    useEditStore.getState().initFromVideo(90);
    useEditStore.getState().splitAt(30); // [0-30], [30-90]
    useEditStore.getState().splitAt(60); // [0-30], [30-60], [60-90]

    useEditStore.getState().setClipSpeed(0, 2.0); // output 15
    useEditStore.getState().setClipSpeed(1, 1.0); // output 30
    useEditStore.getState().setClipSpeed(2, 0.5); // output 60

    expect(useEditStore.getState().getClipOutputStart(0)).toBe(0);
    expect(useEditStore.getState().getClipOutputStart(1)).toBe(15);
    expect(useEditStore.getState().getClipOutputStart(2)).toBe(45); // 15+30
  });

  it("returns 0 for index 0", () => {
    useEditStore.getState().initFromVideo(60);
    expect(useEditStore.getState().getClipOutputStart(0)).toBe(0);
  });
});

describe("outputTimeToSource mapping", () => {
  it("maps correctly at clip boundaries", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().splitAt(30);

    // At output time 0, source is 0
    expect(useEditStore.getState().outputTimeToSource(0)).toBe(0);
    // At output time 30 (boundary), source is start of second clip = 30
    expect(useEditStore.getState().outputTimeToSource(30)).toBe(30);
    // At output time 60, past end
    expect(useEditStore.getState().outputTimeToSource(60)).toBe(60);
  });

  it("maps correctly with speed changes", () => {
    useEditStore.getState().initFromVideo(80);
    useEditStore.getState().splitAt(40); // [0-40], [40-80]
    useEditStore.getState().setClipSpeed(0, 2.0); // output 20s

    // Output time 10 -> in first clip, source = 10 * 2 = 20
    expect(useEditStore.getState().outputTimeToSource(10)).toBe(20);
    // Output time 20 -> boundary, source = 40
    expect(useEditStore.getState().outputTimeToSource(20)).toBe(40);
    // Output time 30 -> in second clip @1x, source = 40 + (30-20)*1 = 50
    expect(useEditStore.getState().outputTimeToSource(30)).toBe(50);
  });

  it("returns 0 for empty clips", () => {
    expect(useEditStore.getState().outputTimeToSource(10)).toBe(0);
  });

  it("returns last clip end when past total duration", () => {
    useEditStore.getState().initFromVideo(60);
    expect(useEditStore.getState().outputTimeToSource(999)).toBe(60);
  });
});

describe("undo/redo", () => {
  it("undo after split restores original single clip", () => {
    useEditStore.getState().initFromVideo(60);
    expect(useEditStore.getState().clips).toHaveLength(1);

    useEditStore.getState().splitAt(30);
    expect(useEditStore.getState().clips).toHaveLength(2);

    useEditStore.getState().undo();
    expect(useEditStore.getState().clips).toHaveLength(1);
    expect(useEditStore.getState().clips[0].sourceStart).toBe(0);
    expect(useEditStore.getState().clips[0].sourceEnd).toBe(60);
  });

  it("undo after delete restores deleted clip", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().splitAt(20);
    expect(useEditStore.getState().clips).toHaveLength(2);

    const clipsBefore = useEditStore.getState().clips.map((c) => ({
      sourceStart: c.sourceStart,
      sourceEnd: c.sourceEnd,
    }));

    useEditStore.getState().deleteClip(0);
    expect(useEditStore.getState().clips).toHaveLength(1);

    useEditStore.getState().undo();
    expect(useEditStore.getState().clips).toHaveLength(2);
    expect(useEditStore.getState().clips[0].sourceStart).toBe(clipsBefore[0].sourceStart);
    expect(useEditStore.getState().clips[0].sourceEnd).toBe(clipsBefore[0].sourceEnd);
    expect(useEditStore.getState().clips[1].sourceStart).toBe(clipsBefore[1].sourceStart);
    expect(useEditStore.getState().clips[1].sourceEnd).toBe(clipsBefore[1].sourceEnd);
  });

  it("redo after undo re-applies the split", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().splitAt(30);
    expect(useEditStore.getState().clips).toHaveLength(2);

    useEditStore.getState().undo();
    expect(useEditStore.getState().clips).toHaveLength(1);

    useEditStore.getState().redo();
    expect(useEditStore.getState().clips).toHaveLength(2);
    expect(useEditStore.getState().clips[0].sourceEnd).toBeCloseTo(30, 5);
    expect(useEditStore.getState().clips[1].sourceStart).toBeCloseTo(30, 5);
  });

  it("multiple undos in sequence", () => {
    useEditStore.getState().initFromVideo(90);
    useEditStore.getState().splitAt(30);   // 2 clips
    useEditStore.getState().splitAt(60);   // 3 clips
    useEditStore.getState().setClipSpeed(0, 2.0);

    expect(useEditStore.getState().clips).toHaveLength(3);
    expect(useEditStore.getState().clips[0].speed).toBe(2.0);

    // Undo speed change
    useEditStore.getState().undo();
    expect(useEditStore.getState().clips[0].speed).toBe(1.0);
    expect(useEditStore.getState().clips).toHaveLength(3);

    // Undo second split
    useEditStore.getState().undo();
    expect(useEditStore.getState().clips).toHaveLength(2);

    // Undo first split
    useEditStore.getState().undo();
    expect(useEditStore.getState().clips).toHaveLength(1);
    expect(useEditStore.getState().clips[0].sourceEnd).toBe(90);
  });

  it("undo stack is cleared on new initFromVideo", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().splitAt(30);
    useEditStore.getState().splitAt(15);
    expect(useEditStore.getState().canUndo()).toBe(true);

    useEditStore.getState().initFromVideo(120);
    expect(useEditStore.getState().canUndo()).toBe(false);
    expect(useEditStore.getState().canRedo()).toBe(false);
    expect(useEditStore.getState().clips).toHaveLength(1);
    expect(useEditStore.getState().clips[0].sourceEnd).toBe(120);
  });

  it("redo is cleared when a new action is performed after undo", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().splitAt(30);
    useEditStore.getState().undo();
    expect(useEditStore.getState().canRedo()).toBe(true);

    // New action clears redo
    useEditStore.getState().splitAt(20);
    expect(useEditStore.getState().canRedo()).toBe(false);
  });

  it("undo does nothing when stack is empty", () => {
    useEditStore.getState().initFromVideo(60);
    const clipsBefore = useEditStore.getState().clips.map((c) => c.id);

    useEditStore.getState().undo();
    const clipsAfter = useEditStore.getState().clips.map((c) => c.id);
    expect(clipsAfter).toEqual(clipsBefore);
  });

  it("redo does nothing when stack is empty", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().splitAt(30);

    const clipsBefore = useEditStore.getState().clips.map((c) => c.id);
    useEditStore.getState().redo();
    const clipsAfter = useEditStore.getState().clips.map((c) => c.id);
    expect(clipsAfter).toEqual(clipsBefore);
  });
});

describe("setClipSpeedLive and commitSpeedChange", () => {
  it("live speed updates don't push to undo stack", () => {
    useEditStore.getState().initFromVideo(60);

    useEditStore.getState().setClipSpeedLive(0, 1.5);
    useEditStore.getState().setClipSpeedLive(0, 1.8);
    useEditStore.getState().setClipSpeedLive(0, 2.0);

    expect(useEditStore.getState().clips[0].speed).toBe(2.0);
    // No undo entries yet (undo stack was empty after init)
    expect(useEditStore.getState().canUndo()).toBe(false);
  });

  it("commitSpeedChange pushes pre-drag snapshot to undo", () => {
    useEditStore.getState().initFromVideo(60);

    useEditStore.getState().setClipSpeedLive(0, 1.5);
    useEditStore.getState().setClipSpeedLive(0, 2.0);
    useEditStore.getState().commitSpeedChange();

    expect(useEditStore.getState().canUndo()).toBe(true);
    expect(useEditStore.getState().clips[0].speed).toBe(2.0);

    useEditStore.getState().undo();
    expect(useEditStore.getState().clips[0].speed).toBe(1.0);
  });
});

describe("setClipSkipFrames and setClipFps", () => {
  it("setClipSkipFrames updates the flag", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().setClipSkipFrames(0, true);
    expect(useEditStore.getState().clips[0].skipFrames).toBe(true);
  });

  it("setClipFps updates fps override", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().setClipFps(0, 24);
    expect(useEditStore.getState().clips[0].fpsOverride).toBe(24);

    useEditStore.getState().setClipFps(0, null);
    expect(useEditStore.getState().clips[0].fpsOverride).toBeNull();
  });
});
