import { describe, it, expect } from "vitest";
import { computeEditPlanHash } from "./editPlanHash";
import type { EditClip, TimelineEffect } from "../stores/editStore";

function clip(overrides: Partial<EditClip> = {}): EditClip {
  return {
    id: "c1",
    sourceStart: 0,
    sourceEnd: 5,
    speed: 1,
    skipFrames: false,
    type: "normal",
    ...overrides,
  } as EditClip;
}

function spotlight(overrides: Partial<TimelineEffect> = {}): TimelineEffect {
  return {
    id: "e1",
    type: "spotlight",
    startTime: 0,
    endTime: 1,
    spotlight: { x: 0.5, y: 0.5, radius: 0.1, dimOpacity: 0.8 },
    ...overrides,
  } as TimelineEffect;
}

function zoomPan(overrides: Partial<TimelineEffect> = {}): TimelineEffect {
  return {
    id: "e2",
    type: "zoom-pan",
    startTime: 0,
    endTime: 1,
    zoomPan: {
      startRegion: { x: 0, y: 0, width: 1, height: 1 },
      endRegion: { x: 0.25, y: 0.25, width: 0.5, height: 0.5 },
      easing: "ease-in-out",
    },
    ...overrides,
  } as TimelineEffect;
}

describe("computeEditPlanHash", () => {
  it("is deterministic for the same input", () => {
    const c = [clip()];
    const e = [spotlight()];
    expect(computeEditPlanHash(c, e)).toBe(computeEditPlanHash(c, e));
  });

  it("changes when a clip field changes", () => {
    const h1 = computeEditPlanHash([clip({ speed: 1 })], []);
    const h2 = computeEditPlanHash([clip({ speed: 2 })], []);
    expect(h1).not.toBe(h2);
  });

  it("changes when an effect's spotlight position changes", () => {
    const h1 = computeEditPlanHash([clip()], [spotlight({ spotlight: { x: 0.5, y: 0.5, radius: 0.1, dimOpacity: 0.8 } })]);
    const h2 = computeEditPlanHash([clip()], [spotlight({ spotlight: { x: 0.8, y: 0.2, radius: 0.1, dimOpacity: 0.8 } })]);
    expect(h1).not.toBe(h2);
  });

  // Regression: previously `computeEditPlanHash` sorted effects by startTime
  // before hashing, so two plans whose effect arrays were in a different order
  // collided — even though the Rust compositor renders them in array order
  // and a swap can visually change the output (e.g. fade-over-text vs
  // text-over-fade, or zoom-pan ordering edge cases pre-two-pass fix).
  it("invalidates the cache when effects are reordered", () => {
    const sp = spotlight();
    const zp = zoomPan();
    const hSpFirst = computeEditPlanHash([clip()], [sp, zp]);
    const hZpFirst = computeEditPlanHash([clip()], [zp, sp]);
    expect(hSpFirst).not.toBe(hZpFirst);
  });

  it("is order-sensitive across identical effect types", () => {
    const a = spotlight({ id: "a", spotlight: { x: 0.1, y: 0.1, radius: 0.1, dimOpacity: 0.5 } });
    const b = spotlight({ id: "b", spotlight: { x: 0.9, y: 0.9, radius: 0.1, dimOpacity: 0.5 } });
    expect(computeEditPlanHash([clip()], [a, b])).not.toBe(computeEditPlanHash([clip()], [b, a]));
  });

  it("returns the same hash for identical plans with different effect IDs", () => {
    // IDs are not part of the render output, so they shouldn't change the hash.
    const e1 = spotlight({ id: "x-1" });
    const e2 = spotlight({ id: "x-2" });
    expect(computeEditPlanHash([clip()], [e1])).toBe(computeEditPlanHash([clip()], [e2]));
  });
});
