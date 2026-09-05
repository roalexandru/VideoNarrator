import { describe, it, expect } from "vitest";
import { removeSilences, DEFAULT_TRIM_OPTIONS } from "./removeSilences";
import type { EditClip } from "../stores/editStore";

function clip(over: Partial<EditClip> = {}): EditClip {
  return {
    id: "c1",
    mediaRefId: "m1",
    sourceStart: 0,
    sourceEnd: 10,
    speed: 1,
    skipFrames: false,
    fpsOverride: null,
    type: "normal",
    ...over,
  };
}

// Zero padding keeps the arithmetic readable; padding gets its own tests.
const NO_PAD = { padding: 0, minSilence: 0.5, minClip: 0.1 };

describe("removeSilences", () => {
  it("cuts a single mid-clip silence into two fragments", () => {
    const r = removeSilences([clip()], [{ start: 4, end: 6 }], "m1", "m1", NO_PAD);
    expect(r.clips).toHaveLength(2);
    expect(r.clips[0]).toMatchObject({ sourceStart: 0, sourceEnd: 4 });
    expect(r.clips[1]).toMatchObject({ sourceStart: 6, sourceEnd: 10 });
    expect(r.removedSeconds).toBeCloseTo(2);
    expect(r.cuts).toBe(1);
  });

  it("gives split fragments fresh ids so React keys and selection don't collide", () => {
    const r = removeSilences([clip()], [{ start: 4, end: 6 }], "m1", "m1", NO_PAD);
    const ids = r.clips.map((c) => c.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("leaves the clip untouched when no silence is long enough", () => {
    const r = removeSilences([clip()], [{ start: 4, end: 4.2 }], "m1", "m1", NO_PAD);
    expect(r.cuts).toBe(0);
    expect(r.clips[0]).toMatchObject({ sourceStart: 0, sourceEnd: 10, id: "c1" });
  });

  it("keeps padding of silence at both edges of a cut", () => {
    const r = removeSilences([clip()], [{ start: 4, end: 6 }], "m1", "m1", {
      padding: 0.25,
      minSilence: 0.5,
      minClip: 0.1,
    });
    expect(r.clips[0].sourceEnd).toBeCloseTo(4.25);
    expect(r.clips[1].sourceStart).toBeCloseTo(5.75);
    expect(r.removedSeconds).toBeCloseTo(1.5);
  });

  it("skips a silence that padding would erase entirely", () => {
    // 0.6s span, 0.4s padding each side -> nothing left to cut.
    const r = removeSilences([clip()], [{ start: 4, end: 4.6 }], "m1", "m1", {
      padding: 0.4,
      minSilence: 0.5,
      minClip: 0.1,
    });
    expect(r.cuts).toBe(0);
    expect(r.clips).toHaveLength(1);
  });

  it("trims a silence that runs off the head and tail of the clip", () => {
    const r = removeSilences(
      [clip({ sourceStart: 2, sourceEnd: 8 })],
      [
        { start: 0, end: 3 },
        { start: 7, end: 12 },
      ],
      "m1",
      "m1",
      NO_PAD,
    );
    expect(r.clips).toHaveLength(1);
    expect(r.clips[0]).toMatchObject({ sourceStart: 3, sourceEnd: 7 });
  });

  it("merges overlapping and unsorted spans instead of double-counting them", () => {
    const r = removeSilences(
      [clip()],
      [
        { start: 6, end: 8 },
        { start: 4, end: 7 },
      ],
      "m1",
      "m1",
      NO_PAD,
    );
    expect(r.clips).toHaveLength(2);
    expect(r.clips[0]).toMatchObject({ sourceStart: 0, sourceEnd: 4 });
    expect(r.clips[1]).toMatchObject({ sourceStart: 8, sourceEnd: 10 });
    expect(r.removedSeconds).toBeCloseTo(4);
  });

  it("normalizes a reversed span rather than producing a negative cut", () => {
    const r = removeSilences([clip()], [{ start: 6, end: 4 }], "m1", "m1", NO_PAD);
    expect(r.clips).toHaveLength(2);
    expect(r.clips[1]).toMatchObject({ sourceStart: 6, sourceEnd: 10 });
  });

  it("ignores non-finite spans", () => {
    const r = removeSilences(
      [clip()],
      [{ start: Number.NaN, end: 5 }, { start: 0, end: Number.POSITIVE_INFINITY }],
      "m1",
      "m1",
      NO_PAD,
    );
    expect(r.clips).toHaveLength(1);
    expect(r.cuts).toBe(0);
  });

  it("drops fragments below minClip", () => {
    // 0-2 survives at 2s; the 9.8-10 tail is a 0.2s sliver under the 0.5s floor.
    const r = removeSilences([clip()], [{ start: 2, end: 9.8 }], "m1", "m1", {
      padding: 0,
      minSilence: 0.5,
      minClip: 0.5,
    });
    expect(r.clips).toHaveLength(1);
    expect(r.clips[0]).toMatchObject({ sourceStart: 0, sourceEnd: 2 });
    // The sliver counts as removed time even though it was not silence.
    expect(r.removedSeconds).toBeCloseTo(8);
  });

  it("reports nothing dropped when the whole timeline would vanish", () => {
    // Both fragments fall under the floor, so the no-empty-timeline guarantee
    // returns the original untouched — and must not claim a drop it undid.
    const r = removeSilences([clip()], [{ start: 0.2, end: 9.8 }], "m1", "m1", {
      padding: 0,
      minSilence: 0.5,
      minClip: 0.5,
    });
    expect(r.clips).toHaveLength(1);
    expect(r.clips[0].id).toBe("c1");
    expect(r).toMatchObject({ cuts: 0, removedSeconds: 0, clipsDropped: 0 });
  });

  it("drops a clip that is silence end to end when another clip survives", () => {
    const r = removeSilences(
      [clip({ id: "a", sourceStart: 0, sourceEnd: 10 }), clip({ id: "b", sourceStart: 20, sourceEnd: 30 })],
      [{ start: 0, end: 10 }],
      "m1",
      "m1",
      NO_PAD,
    );
    expect(r.clips).toHaveLength(1);
    expect(r.clips[0].id).toBe("b");
    expect(r.clipsDropped).toBe(1);
  });

  it("never returns an empty timeline", () => {
    const r = removeSilences([clip()], [{ start: 0, end: 10 }], "m1", "m1", NO_PAD);
    expect(r.clips).toHaveLength(1);
    expect(r.cuts).toBe(0);
    expect(r.clips[0].id).toBe("c1");
  });

  it("only touches clips pointing at the analyzed media", () => {
    const r = removeSilences(
      [clip({ id: "a", mediaRefId: "m1" }), clip({ id: "b", mediaRefId: "m2" })],
      [{ start: 4, end: 6 }],
      "m1",
      "m1",
      NO_PAD,
    );
    // "a" split in two, "b" untouched.
    expect(r.clips).toHaveLength(3);
    expect(r.clips.find((c) => c.id === "b")).toMatchObject({ sourceStart: 0, sourceEnd: 10 });
  });

  it("treats a null mediaRefId as the primary media", () => {
    const r = removeSilences([clip({ mediaRefId: null })], [{ start: 4, end: 6 }], "m1", "m1", NO_PAD);
    expect(r.clips).toHaveLength(2);
  });

  it("leaves freeze and image clips alone", () => {
    const r = removeSilences(
      [
        clip({ id: "f", type: "freeze", freezeSourceTime: 5, freezeDuration: 3 }),
        clip({ id: "i", type: "image", imageDuration: 4 }),
      ],
      [{ start: 0, end: 10 }],
      "m1",
      "m1",
      NO_PAD,
    );
    expect(r.clips.map((c) => c.id)).toEqual(["f", "i"]);
    expect(r.cuts).toBe(0);
  });

  it("preserves speed, skipFrames and fpsOverride on every fragment", () => {
    const r = removeSilences(
      [clip({ speed: 2, skipFrames: true, fpsOverride: 24 })],
      [{ start: 4, end: 6 }],
      "m1",
      "m1",
      NO_PAD,
    );
    expect(r.clips).toHaveLength(2);
    for (const c of r.clips) {
      expect(c).toMatchObject({ speed: 2, skipFrames: true, fpsOverride: 24 });
    }
  });

  it("drops legacy zoomPan when a clip is split but keeps it when untouched", () => {
    const zoomPan = { startRegion: { x: 0, y: 0, w: 1, h: 1 }, endRegion: { x: 0, y: 0, w: 0.5, h: 0.5 }, easing: "linear" } as any;
    const split = removeSilences([clip({ zoomPan })], [{ start: 4, end: 6 }], "m1", "m1", NO_PAD);
    expect(split.clips).toHaveLength(2);
    expect(split.clips.every((c) => c.zoomPan === undefined)).toBe(true);

    const untouched = removeSilences([clip({ zoomPan })], [{ start: 40, end: 60 }], "m1", "m1", NO_PAD);
    expect(untouched.clips[0].zoomPan).toBe(zoomPan);
  });

  it("is a no-op for an empty span list", () => {
    const r = removeSilences([clip()], [], "m1", "m1", NO_PAD);
    expect(r).toMatchObject({ cuts: 0, removedSeconds: 0, clipsDropped: 0 });
    expect(r.clips[0].id).toBe("c1");
  });

  it("ships sane defaults", () => {
    expect(DEFAULT_TRIM_OPTIONS.minSilence).toBeGreaterThan(DEFAULT_TRIM_OPTIONS.padding * 2);
    expect(DEFAULT_TRIM_OPTIONS.minClip).toBeGreaterThan(0);
  });
});
