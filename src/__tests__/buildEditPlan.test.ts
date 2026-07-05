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
import {
  buildEditPlan,
  editsRequireRender,
  resolveEditedVideo,
} from "../lib/buildEditPlan";
import { computeEditPlanHash } from "../lib/editPlanHash";
import type { EditClip, TimelineEffect, EffectType } from "../stores/editStore";

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

function mkEffect(type: EffectType, overrides: Partial<TimelineEffect> = {}): TimelineEffect {
  return {
    id: `e-${type}`,
    type,
    startTime: 1,
    endTime: 3,
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

describe("buildEditPlan — skip_frames serialization", () => {
  it("serializes skip_frames=true for time-lapse clips", () => {
    const plan = buildEditPlan([mkClip({ speed: 4, skipFrames: true })], []);
    expect(plan.clips[0].skip_frames).toBe(true);
  });

  it("serializes skip_frames=false by default (field present, not omitted)", () => {
    const plan = buildEditPlan([mkClip()], []);
    expect(plan.clips[0].skip_frames).toBe(false);
  });
});

describe("editsRequireRender", () => {
  it("is false for empty clips", () => {
    expect(editsRequireRender([], [], 10)).toBe(false);
  });

  it("is false for a single full-coverage clip, speed 1, no effects", () => {
    expect(editsRequireRender([mkClip({ sourceStart: 0, sourceEnd: 10 })], [], 10)).toBe(false);
  });

  it("is true for a trim-only edit (end trimmed)", () => {
    expect(editsRequireRender([mkClip({ sourceStart: 0, sourceEnd: 8 })], [], 10)).toBe(true);
  });

  it("is true when sourceStart is trimmed", () => {
    expect(editsRequireRender([mkClip({ sourceStart: 2, sourceEnd: 10 })], [], 10)).toBe(true);
  });

  it("respects the 0.5s trim tolerance", () => {
    expect(editsRequireRender([mkClip({ sourceStart: 0, sourceEnd: 9.6 })], [], 10)).toBe(false);
    expect(editsRequireRender([mkClip({ sourceStart: 0, sourceEnd: 9.4 })], [], 10)).toBe(true);
  });

  it("is true for any effect type (privacy: non-zoom effects must render)", () => {
    const full = mkClip({ sourceStart: 0, sourceEnd: 10 });
    for (const t of ["blur", "text", "spotlight", "fade", "zoom-pan"] as EffectType[]) {
      expect(editsRequireRender([full], [mkEffect(t)], 10)).toBe(true);
    }
  });

  it("is true for speed change / multi-clip / freeze / fpsOverride / skipFrames / clip zoomPan", () => {
    expect(editsRequireRender([mkClip({ speed: 2 })], [], 10)).toBe(true);
    expect(editsRequireRender([mkClip(), mkClip({ id: "c2" })], [], 10)).toBe(true);
    expect(editsRequireRender([mkClip({ type: "freeze" })], [], 10)).toBe(true);
    expect(editsRequireRender([mkClip({ fpsOverride: 30 })], [], 10)).toBe(true);
    expect(editsRequireRender([mkClip({ skipFrames: true })], [], 10)).toBe(true);
    expect(
      editsRequireRender(
        [mkClip({ zoomPan: { startRegion: { x: 0, y: 0, width: 1, height: 1 }, endRegion: { x: 0, y: 0, width: 0.5, height: 0.5 }, easing: "ease-out" } })],
        [],
        10,
      ),
    ).toBe(true);
  });

  it("fails open (no render) when sourceDuration is unknown even if the clip looks trimmed", () => {
    expect(editsRequireRender([mkClip({ sourceStart: 0, sourceEnd: 8 })], [], 0)).toBe(false);
  });
});

describe("resolveEditedVideo", () => {
  const ORIG = "/tmp/orig.mp4";
  const CACHED = "/tmp/edited.mp4";

  it("returns original and ignores a stale editedVideoPath when no render is required", async () => {
    const clips = [mkClip({ sourceStart: 0, sourceEnd: 10 })];
    const r = await resolveEditedVideo({
      clips,
      effects: [],
      sourceDuration: 10,
      originalVideoPath: ORIG,
      editedVideoPath: CACHED, // lingering, should be ignored
      editedVideoPlanHash: "whatever",
      fileExists: async () => true,
    });
    expect(r).toEqual({ kind: "original", path: ORIG });
  });

  it("returns rendered when the hash matches and the file exists", async () => {
    const clips = [mkClip({ speed: 2 })];
    const effects: TimelineEffect[] = [];
    const r = await resolveEditedVideo({
      clips,
      effects,
      sourceDuration: 10,
      originalVideoPath: ORIG,
      editedVideoPath: CACHED,
      editedVideoPlanHash: computeEditPlanHash(clips, effects),
      fileExists: async () => true,
    });
    expect(r).toEqual({ kind: "rendered", path: CACHED });
  });

  it("returns render-required when the hash matches but the file is missing", async () => {
    const clips = [mkClip({ speed: 2 })];
    const effects: TimelineEffect[] = [];
    const r = await resolveEditedVideo({
      clips,
      effects,
      sourceDuration: 10,
      originalVideoPath: ORIG,
      editedVideoPath: CACHED,
      editedVideoPlanHash: computeEditPlanHash(clips, effects),
      fileExists: async () => false,
    });
    expect(r).toEqual({ kind: "render-required" });
  });

  it("returns render-required when the hash mismatches", async () => {
    const r = await resolveEditedVideo({
      clips: [mkClip({ speed: 2 })],
      effects: [],
      sourceDuration: 10,
      originalVideoPath: ORIG,
      editedVideoPath: CACHED,
      editedVideoPlanHash: "stale-hash",
      fileExists: async () => true,
    });
    expect(r).toEqual({ kind: "render-required" });
  });

  it("returns render-required for a trim-only plan with no cache", async () => {
    const r = await resolveEditedVideo({
      clips: [mkClip({ sourceStart: 0, sourceEnd: 6 })],
      effects: [],
      sourceDuration: 10,
      originalVideoPath: ORIG,
      editedVideoPath: null,
      editedVideoPlanHash: null,
      fileExists: async () => false,
    });
    expect(r).toEqual({ kind: "render-required" });
  });
});
