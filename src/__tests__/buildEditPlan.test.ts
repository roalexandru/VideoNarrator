/**
 * buildEditPlan: the frontend-side plan builder that the three render-
 * triggering screens (EditVideoScreen / ProcessingScreen / ExportScreen)
 * all go through. Important invariants:
 *
 *   - Clips from the project's PRIMARY MediaRef are sent without an
 *     `input_path` field (so the Rust `apply_edits(input_path, ...)`
 *     fallback is exercised — matches legacy behavior + keeps the editPlan
 *     hash stable for already-rendered projects).
 *
 *   - Clips from secondary MediaRefs send their own `input_path`.
 *
 *   - Image clips send `image_duration` and `clip_type: "image"`.
 */

import { describe, it, expect } from "vitest";
import { buildEditPlan } from "../lib/buildEditPlan";
import type { EditClip } from "../stores/editStore";

const PRIMARY = "primary";
const SECOND = "second";

function mkClip(overrides: Partial<EditClip> = {}): EditClip {
  return {
    id: "c1",
    mediaRefId: PRIMARY,
    sourceStart: 0,
    sourceEnd: 10,
    speed: 1,
    skipFrames: false,
    fpsOverride: null,
    ...overrides,
  };
}

describe("buildEditPlan — multi-source", () => {
  it("omits input_path for primary-sourced clips", () => {
    const plan = buildEditPlan(
      [mkClip()],
      [],
      (_c) => ({ id: PRIMARY, path: "/tmp/primary.mp4" }),
      PRIMARY,
    );
    expect(plan.clips[0].input_path).toBeUndefined();
  });

  it("emits input_path for secondary-sourced clips", () => {
    const plan = buildEditPlan(
      [mkClip({ mediaRefId: SECOND })],
      [],
      (c) =>
        c.mediaRefId === SECOND
          ? { id: SECOND, path: "/tmp/other.mp4" }
          : { id: PRIMARY, path: "/tmp/primary.mp4" },
      PRIMARY,
    );
    expect(plan.clips[0].input_path).toBe("/tmp/other.mp4");
  });

  it("emits clip_type=image and image_duration for image clips", () => {
    const plan = buildEditPlan(
      [
        mkClip({
          mediaRefId: "img1",
          type: "image",
          imageDuration: 4.5,
          sourceStart: 0,
          sourceEnd: 0,
        }),
      ],
      [],
      (_c) => ({ id: "img1", path: "/tmp/a.png" }),
      PRIMARY,
    );
    expect(plan.clips[0].clip_type).toBe("image");
    expect(plan.clips[0].image_duration).toBe(4.5);
    expect(plan.clips[0].input_path).toBe("/tmp/a.png");
  });

  it("legacy clips without mediaRefId are treated as primary (no input_path)", () => {
    // resolveClipMedia returns primary for a clip with undefined mediaRefId
    const legacy: EditClip = {
      id: "c1",
      sourceStart: 0,
      sourceEnd: 10,
      speed: 1,
      skipFrames: false,
      fpsOverride: null,
    };
    const plan = buildEditPlan(
      [legacy],
      [],
      (_c) => ({ id: PRIMARY, path: "/tmp/primary.mp4" }),
      PRIMARY,
    );
    expect(plan.clips[0].input_path).toBeUndefined();
  });

  it("back-compat call without resolver still emits a valid plan (no input_path, no image fields)", () => {
    const plan = buildEditPlan([mkClip()], []);
    expect(plan.clips[0].input_path).toBeUndefined();
    expect(plan.clips[0].clip_type).toBe("normal");
  });
});
