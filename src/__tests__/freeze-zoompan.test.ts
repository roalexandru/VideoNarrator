import { describe, it, expect, beforeEach } from "vitest";
import { setupDefaultMocks, resetAllStores } from "./setup";
import { useEditStore } from "../stores/editStore";
import type { ZoomPanEffect, TimelineEffect } from "../stores/editStore";
import { applyEasing, computeZoomTransform, computeZoomAtTime } from "../features/edit-video/easing";

beforeEach(() => {
  setupDefaultMocks();
  resetAllStores();
});

// ── Freeze Frame ──

describe("insertFreezeFrame", () => {
  it("inserts a freeze clip at the middle of a clip, splitting it", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().insertFreezeFrame(30);
    const { clips, selectedClipIndex } = useEditStore.getState();
    expect(clips).toHaveLength(3);
    // First half: 0-30
    expect(clips[0].sourceStart).toBe(0);
    expect(clips[0].sourceEnd).toBe(30);
    // Freeze clip in the middle
    expect(clips[1].type).toBe("freeze");
    expect(clips[1].freezeSourceTime).toBe(30);
    expect(clips[1].freezeDuration).toBe(3);
    // Second half: 30-60
    expect(clips[2].sourceStart).toBe(30);
    expect(clips[2].sourceEnd).toBe(60);
    // Selected clip should be the freeze
    expect(selectedClipIndex).toBe(1);
  });

  it("inserts freeze with custom duration", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().insertFreezeFrame(10, 5);
    const freeze = useEditStore.getState().clips[1];
    expect(freeze.type).toBe("freeze");
    expect(freeze.freezeDuration).toBe(5);
  });

  it("inserts at clip boundary (appends after current clip)", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().insertFreezeFrame(0);
    const { clips } = useEditStore.getState();
    // At the very start — inserted after the clip
    expect(clips.length).toBeGreaterThanOrEqual(2);
    const freezeClips = clips.filter((c) => c.type === "freeze");
    expect(freezeClips).toHaveLength(1);
  });

  it("pushes undo state", () => {
    useEditStore.getState().initFromVideo(60);
    expect(useEditStore.getState().canUndo()).toBe(false);
    useEditStore.getState().insertFreezeFrame(30);
    expect(useEditStore.getState().canUndo()).toBe(true);
    useEditStore.getState().undo();
    expect(useEditStore.getState().clips).toHaveLength(1);
  });
});

describe("setFreezeDuration", () => {
  it("updates freeze clip duration", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().insertFreezeFrame(30);
    const freezeIdx = useEditStore.getState().clips.findIndex((c) => c.type === "freeze");
    useEditStore.getState().setFreezeDuration(freezeIdx, 7);
    expect(useEditStore.getState().clips[freezeIdx].freezeDuration).toBe(7);
  });

  it("clamps duration to minimum 0.1", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().insertFreezeFrame(30);
    const freezeIdx = useEditStore.getState().clips.findIndex((c) => c.type === "freeze");
    useEditStore.getState().setFreezeDuration(freezeIdx, 0);
    expect(useEditStore.getState().clips[freezeIdx].freezeDuration).toBe(0.1);
  });

  it("does nothing for non-freeze clips", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().setFreezeDuration(0, 5);
    // Normal clip, no type, no change
    expect(useEditStore.getState().clips[0].freezeDuration).toBeUndefined();
  });
});

describe("freeze frame in calculations", () => {
  it("getOutputDuration includes freeze duration", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().insertFreezeFrame(30);
    // 30s (left) + 3s (freeze) + 30s (right) = 63s
    expect(useEditStore.getState().getOutputDuration()).toBeCloseTo(63, 1);
  });

  it("outputTimeToSource returns freezeSourceTime for freeze clips", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().insertFreezeFrame(30);
    // At output time 31 (inside the freeze clip, which starts at 30 and lasts 3s)
    const sourceT = useEditStore.getState().outputTimeToSource(31);
    expect(sourceT).toBe(30);
  });

  it("splitAt skips freeze clips", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().insertFreezeFrame(30);
    const before = useEditStore.getState().clips.length;
    // Try to split at a time inside the freeze clip (30-33 output time)
    useEditStore.getState().splitAt(31);
    // Should not add clips — freeze clips can't be split
    expect(useEditStore.getState().clips).toHaveLength(before);
  });

  it("getClipOutputStart accounts for freeze clips", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().insertFreezeFrame(30);
    // Clip 0 starts at 0, clip 1 (freeze) starts at 30, clip 2 starts at 33
    expect(useEditStore.getState().getClipOutputStart(0)).toBe(0);
    expect(useEditStore.getState().getClipOutputStart(1)).toBeCloseTo(30, 1);
    expect(useEditStore.getState().getClipOutputStart(2)).toBeCloseTo(33, 1);
  });
});

// ── Zoom/Pan ──

const sampleZoomPan: ZoomPanEffect = {
  startRegion: { x: 0, y: 0, width: 1, height: 1 },
  endRegion: { x: 0.25, y: 0.25, width: 0.5, height: 0.5 },
  easing: "ease-in-out",
};

describe("setClipZoomPan", () => {
  it("sets zoom/pan effect on a clip", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().setClipZoomPan(0, sampleZoomPan);
    const clip = useEditStore.getState().clips[0];
    expect(clip.zoomPan).not.toBeNull();
    expect(clip.zoomPan!.startRegion.width).toBe(1);
    expect(clip.zoomPan!.endRegion.width).toBe(0.5);
    expect(clip.zoomPan!.easing).toBe("ease-in-out");
  });

  it("clears zoom/pan when set to null", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().setClipZoomPan(0, sampleZoomPan);
    useEditStore.getState().setClipZoomPan(0, null);
    expect(useEditStore.getState().clips[0].zoomPan).toBeNull();
  });

  it("pushes undo state", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().setClipZoomPan(0, sampleZoomPan);
    expect(useEditStore.getState().canUndo()).toBe(true);
    useEditStore.getState().undo();
    expect(useEditStore.getState().clips[0].zoomPan).toBeUndefined();
  });
});

describe("setClipZoomPanLive + commitZoomPanChange", () => {
  it("updates zoom/pan without pushing undo during drag", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().setClipZoomPanLive(0, sampleZoomPan);
    expect(useEditStore.getState().clips[0].zoomPan).not.toBeNull();
    expect(useEditStore.getState().canUndo()).toBe(false);
  });

  it("commits undo on release", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().setClipZoomPanLive(0, sampleZoomPan);
    useEditStore.getState().commitZoomPanChange();
    expect(useEditStore.getState().canUndo()).toBe(true);
  });
});

describe("setClipEasing", () => {
  it("changes the easing preset", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().setClipZoomPan(0, sampleZoomPan);
    useEditStore.getState().setClipEasing(0, "linear");
    expect(useEditStore.getState().clips[0].zoomPan!.easing).toBe("linear");
  });

  it("does nothing if clip has no zoom/pan", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().setClipEasing(0, "ease-in");
    expect(useEditStore.getState().clips[0].zoomPan).toBeUndefined();
  });
});

// ── Easing Utility ──

describe("applyEasing", () => {
  it("linear returns input unchanged", () => {
    expect(applyEasing(0, "linear")).toBe(0);
    expect(applyEasing(0.5, "linear")).toBe(0.5);
    expect(applyEasing(1, "linear")).toBe(1);
  });

  it("ease-in starts slow (below linear at midpoint)", () => {
    expect(applyEasing(0.5, "ease-in")).toBeLessThan(0.5);
  });

  it("ease-out starts fast (above linear at midpoint)", () => {
    expect(applyEasing(0.5, "ease-out")).toBeGreaterThan(0.5);
  });

  it("ease-in-out is 0.5 at midpoint", () => {
    expect(applyEasing(0.5, "ease-in-out")).toBeCloseTo(0.5, 5);
  });

  it("all presets reach 0 at start and 1 at end", () => {
    for (const preset of ["linear", "ease-in", "ease-out", "ease-in-out"] as const) {
      expect(applyEasing(0, preset)).toBe(0);
      expect(applyEasing(1, preset)).toBeCloseTo(1, 5);
    }
  });

  it("clamps values outside 0-1", () => {
    expect(applyEasing(-0.5, "linear")).toBe(0);
    expect(applyEasing(1.5, "linear")).toBe(1);
  });
});

describe("computeZoomTransform", () => {
  it("returns identity when no zoom/pan", () => {
    const clip = { id: "1", sourceStart: 0, sourceEnd: 60, speed: 1, skipFrames: false, fpsOverride: null };
    const result = computeZoomTransform(clip, 30, 800, 600);
    expect(result.scale).toBe(1);
    expect(result.tx).toBe(0);
    expect(result.ty).toBe(0);
  });

  it("returns zoomed transform at midpoint with zoom/pan", () => {
    const clip = {
      id: "1", sourceStart: 0, sourceEnd: 60, speed: 1, skipFrames: false, fpsOverride: null,
      zoomPan: { ...sampleZoomPan, easing: "linear" as const },
    };
    const result = computeZoomTransform(clip, 30, 800, 600);
    // At midpoint with linear easing, region should be interpolated halfway
    expect(result.scale).toBeGreaterThan(1);
  });

  it("at t=0, uses start region", () => {
    const clip = {
      id: "1", sourceStart: 0, sourceEnd: 60, speed: 1, skipFrames: false, fpsOverride: null,
      zoomPan: {
        startRegion: { x: 0, y: 0, width: 1, height: 1 },
        endRegion: { x: 0.25, y: 0.25, width: 0.5, height: 0.5 },
        easing: "linear" as const,
      },
    };
    const result = computeZoomTransform(clip, 0, 800, 600);
    // Full region = scale 1, no translate
    expect(result.scale).toBeCloseTo(1, 2);
    expect(result.tx).toBeCloseTo(0, 1);
    expect(result.ty).toBeCloseTo(0, 1);
  });

  it("at t=end, uses end region", () => {
    const clip = {
      id: "1", sourceStart: 0, sourceEnd: 60, speed: 1, skipFrames: false, fpsOverride: null,
      zoomPan: {
        startRegion: { x: 0, y: 0, width: 1, height: 1 },
        endRegion: { x: 0.25, y: 0.25, width: 0.5, height: 0.5 },
        easing: "linear" as const,
      },
    };
    const result = computeZoomTransform(clip, 60, 800, 600);
    // End region: 0.5 width = 2x scale
    expect(result.scale).toBeCloseTo(2, 1);
  });
});

// ── Mixed timeline (normal + freeze + zoom/pan) ──

describe("mixed timeline operations", () => {
  it("handles a timeline with normal, freeze, and zoom clips", () => {
    const store = useEditStore.getState();
    store.initFromVideo(60);
    // Insert freeze at 20s
    store.insertFreezeFrame(20, 2);
    // Add zoom to the first clip (0-20s)
    useEditStore.getState().setClipZoomPan(0, sampleZoomPan);

    const { clips } = useEditStore.getState();
    expect(clips).toHaveLength(3);
    expect(clips[0].zoomPan).not.toBeNull();
    expect(clips[1].type).toBe("freeze");
    expect(clips[1].freezeDuration).toBe(2);
    // Total: 20 + 2 + 40 = 62
    expect(useEditStore.getState().getOutputDuration()).toBeCloseTo(62, 1);
  });

  it("undo reverts freeze frame insertion", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().insertFreezeFrame(30);
    expect(useEditStore.getState().clips).toHaveLength(3);
    useEditStore.getState().undo();
    expect(useEditStore.getState().clips).toHaveLength(1);
    expect(useEditStore.getState().getOutputDuration()).toBeCloseTo(60, 1);
  });

  it("redo restores freeze frame after undo", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().insertFreezeFrame(30);
    useEditStore.getState().undo();
    useEditStore.getState().redo();
    expect(useEditStore.getState().clips).toHaveLength(3);
    expect(useEditStore.getState().clips[1].type).toBe("freeze");
  });

  it("zoom/pan on freeze clip works", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().insertFreezeFrame(30);
    const freezeIdx = useEditStore.getState().clips.findIndex((c) => c.type === "freeze");
    useEditStore.getState().setClipZoomPan(freezeIdx, sampleZoomPan);
    expect(useEditStore.getState().clips[freezeIdx].zoomPan).not.toBeNull();
  });

  it("delete removes freeze clips", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().insertFreezeFrame(30);
    const freezeIdx = useEditStore.getState().clips.findIndex((c) => c.type === "freeze");
    useEditStore.getState().deleteClip(freezeIdx);
    expect(useEditStore.getState().clips.filter((c) => c.type === "freeze")).toHaveLength(0);
  });
});

// ── Effects Track ──

describe("effects track", () => {
  it("addEffect adds a zoom-pan effect", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().addEffect({
      type: "zoom-pan",
      startTime: 10,
      endTime: 20,
      zoomPan: sampleZoomPan,
    });
    const { effects } = useEditStore.getState();
    expect(effects).toHaveLength(1);
    expect(effects[0].type).toBe("zoom-pan");
    expect(effects[0].startTime).toBe(10);
    expect(effects[0].endTime).toBe(20);
    expect(effects[0].id).toBeTruthy();
  });

  it("removeEffect removes an effect", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().addEffect({ type: "zoom-pan", startTime: 5, endTime: 15, zoomPan: sampleZoomPan });
    const id = useEditStore.getState().effects[0].id;
    useEditStore.getState().removeEffect(id);
    expect(useEditStore.getState().effects).toHaveLength(0);
  });

  it("updateEffect updates effect properties", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().addEffect({ type: "zoom-pan", startTime: 5, endTime: 15, zoomPan: sampleZoomPan });
    const id = useEditStore.getState().effects[0].id;
    useEditStore.getState().updateEffect(id, { endTime: 25 });
    expect(useEditStore.getState().effects[0].endTime).toBe(25);
  });

  it("selectEffect selects an effect and deselects clip", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().addEffect({ type: "zoom-pan", startTime: 5, endTime: 15, zoomPan: sampleZoomPan });
    const id = useEditStore.getState().effects[0].id;
    useEditStore.getState().selectEffect(id);
    expect(useEditStore.getState().selectedEffectId).toBe(id);
  });

  it("getEffectsAtTime returns effects covering a given time", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().addEffect({ type: "zoom-pan", startTime: 10, endTime: 20, zoomPan: sampleZoomPan });
    expect(useEditStore.getState().getEffectsAtTime(15)).toHaveLength(1);
    expect(useEditStore.getState().getEffectsAtTime(5)).toHaveLength(0);
    expect(useEditStore.getState().getEffectsAtTime(25)).toHaveLength(0);
  });

  it("undo/redo works with effects", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().addEffect({ type: "zoom-pan", startTime: 5, endTime: 15, zoomPan: sampleZoomPan });
    expect(useEditStore.getState().effects).toHaveLength(1);
    useEditStore.getState().undo();
    expect(useEditStore.getState().effects).toHaveLength(0);
    useEditStore.getState().redo();
    expect(useEditStore.getState().effects).toHaveLength(1);
  });

  it("initFromVideo resets effects", () => {
    useEditStore.getState().initFromVideo(60);
    useEditStore.getState().addEffect({ type: "zoom-pan", startTime: 5, endTime: 15, zoomPan: sampleZoomPan });
    useEditStore.getState().initFromVideo(120);
    expect(useEditStore.getState().effects).toHaveLength(0);
  });
});

// ── computeZoomAtTime ──

describe("computeZoomAtTime", () => {
  it("returns identity when no effects", () => {
    const result = computeZoomAtTime([], 10, 800, 600);
    expect(result.scale).toBe(1);
    expect(result.tx).toBe(0);
  });

  it("returns zoom transform when inside an effect", () => {
    const effects: TimelineEffect[] = [{
      id: "1", type: "zoom-pan", startTime: 10, endTime: 20,
      zoomPan: { ...sampleZoomPan, easing: "linear" },
    }];
    const result = computeZoomAtTime(effects, 15, 800, 600);
    expect(result.scale).toBeGreaterThan(1);
  });

  it("returns identity when outside effect time range", () => {
    const effects: TimelineEffect[] = [{
      id: "1", type: "zoom-pan", startTime: 10, endTime: 20,
      zoomPan: { ...sampleZoomPan, easing: "linear" },
    }];
    expect(computeZoomAtTime(effects, 5, 800, 600).scale).toBe(1);
    expect(computeZoomAtTime(effects, 25, 800, 600).scale).toBe(1);
  });
});
