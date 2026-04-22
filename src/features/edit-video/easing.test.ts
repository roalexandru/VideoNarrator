import { describe, it, expect } from "vitest";
import { lockRegionAspect } from "./easing";

describe("lockRegionAspect", () => {
  it("leaves a square region unchanged", () => {
    const r = { x: 0.2, y: 0.3, width: 0.4, height: 0.4 };
    expect(lockRegionAspect(r)).toBe(r);
  });

  it("expands a tall region to the larger side, preserving center when possible", () => {
    // Original center (0.5, 0.5) — expanding to 0.6×0.6 stays inside [0,1].
    const r = { x: 0.4, y: 0.2, width: 0.2, height: 0.6 };
    const locked = lockRegionAspect(r);
    expect(locked.width).toBe(0.6);
    expect(locked.height).toBe(0.6);
    const cx = locked.x + locked.width / 2;
    const cy = locked.y + locked.height / 2;
    expect(cx).toBeCloseTo(0.5, 5);
    expect(cy).toBeCloseTo(0.5, 5);
  });

  it("shifts the rect inward when snapping would push it outside [0,1]", () => {
    // Original center (0.2, 0.5) — expanding to 0.6×0.6 would want x=-0.1,
    // which we clamp to x=0. Center shifts right as a consequence; the
    // invariant "region stays inside the source" wins over exact centering.
    const r = { x: 0.1, y: 0.2, width: 0.2, height: 0.6 };
    const locked = lockRegionAspect(r);
    expect(locked.width).toBe(0.6);
    expect(locked.height).toBe(0.6);
    expect(locked.x).toBe(0);
    expect(locked.x + locked.width).toBeLessThanOrEqual(1);
  });

  it("expands a wide region to the larger side, preserving center", () => {
    const r = { x: 0.1, y: 0.4, width: 0.8, height: 0.2 };
    const locked = lockRegionAspect(r);
    expect(locked.width).toBe(0.8);
    expect(locked.height).toBe(0.8);
    const cx = locked.x + locked.width / 2;
    const cy = locked.y + locked.height / 2;
    expect(cx).toBeCloseTo(0.5, 5);
    expect(cy).toBeCloseTo(0.5, 5);
  });

  it("clamps the snapped region to the source bounds", () => {
    // Center near the top-right corner. Expanding would push the rect out.
    const r = { x: 0.8, y: 0.0, width: 0.15, height: 0.4 };
    const locked = lockRegionAspect(r);
    expect(locked.width).toBe(0.4);
    expect(locked.height).toBe(0.4);
    // Right edge should not exceed 1.
    expect(locked.x + locked.width).toBeLessThanOrEqual(1.0001);
    // Top edge should not go below 0.
    expect(locked.y).toBeGreaterThanOrEqual(0);
  });

  it("caps size at the full source (1.0)", () => {
    const r = { x: 0, y: 0, width: 1.2, height: 0.9 };
    const locked = lockRegionAspect(r);
    expect(locked.width).toBe(1.0);
    expect(locked.height).toBe(1.0);
  });

  it("returns a safe fallback for non-finite input", () => {
    const locked = lockRegionAspect({ x: 0, y: 0, width: NaN, height: 0.5 });
    expect(locked.width).toBe(1);
    expect(locked.height).toBe(1);
  });

  it("is idempotent", () => {
    const r = { x: 0.1, y: 0.2, width: 0.2, height: 0.6 };
    const once = lockRegionAspect(r);
    const twice = lockRegionAspect(once);
    expect(twice).toEqual(once);
  });
});
