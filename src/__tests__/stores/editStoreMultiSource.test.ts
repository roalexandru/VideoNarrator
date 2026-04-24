/**
 * Multi-source + image clip tests. These are the tests that lock down the
 * fix for the "playhead stuck on clip 3" bug — specifically:
 *
 *   - `resolveAtOutputTime` must map output time → source time PER-CLIP,
 *     not via a global `sourceT → clip` inverse. So two clips with
 *     OVERLAPPING source ranges (e.g. two uses of different video files
 *     that both start at 0) must resolve to their respective clips and
 *     mediaRefs, not collide.
 *
 *   - Image clips use `imageDuration` for output duration, `sourceStart/End`
 *     are ignored.
 *
 *   - The primary MediaRef fallback still works for legacy clips with
 *     `mediaRefId == null`.
 */

import { describe, it, expect, beforeEach } from "vitest";
import { useEditStore, PRIMARY_MEDIA_REF_ID } from "../../stores/editStore";

describe("multi-source resolver", () => {
  beforeEach(() => {
    useEditStore.getState().reset();
  });

  it("resolves per-clip when two clips point at different MediaRefs with overlapping source ranges", () => {
    // This is the regression test for the user-reported bug: two "+"-added
    // videos both with sourceStart=0 would collide in the old outputTimeToSource.
    const s = useEditStore.getState();
    s.registerMedia({
      id: "primary",
      hash: "primary",
      kind: "video",
      path: "/tmp/primary.mp4",
      duration: 30,
      width: 1920,
      height: 1080,
    });
    s.setPrimaryMediaRef({
      id: "primary",
      hash: "primary",
      kind: "video",
      path: "/tmp/primary.mp4",
      duration: 30,
      width: 1920,
      height: 1080,
    });
    s.registerMedia({
      id: "second",
      hash: "second",
      kind: "video",
      path: "/tmp/second.mp4",
      duration: 30,
      width: 1920,
      height: 1080,
    });

    // Clip 1: primary, 0..10s @ 1x → output 0..10
    useEditStore.setState({
      clips: [
        { id: "c1", mediaRefId: "primary", sourceStart: 0, sourceEnd: 10, speed: 1, skipFrames: false, fpsOverride: null },
        { id: "c2", mediaRefId: "second", sourceStart: 0, sourceEnd: 10, speed: 1, skipFrames: false, fpsOverride: null },
      ],
    });

    // At output 5s we're in clip 1 — primary.
    const a = useEditStore.getState().resolveAtOutputTime(5);
    expect(a).not.toBeNull();
    expect(a!.clipIndex).toBe(0);
    expect(a!.mediaRef?.id).toBe("primary");
    expect(a!.sourceTime).toBeCloseTo(5, 4);

    // At output 12s we're in clip 2 — SECOND, with sourceTime 2 (in its own file),
    // NOT in clip 1 (which would be the bug — primary.mp4's t=2s).
    const b = useEditStore.getState().resolveAtOutputTime(12);
    expect(b).not.toBeNull();
    expect(b!.clipIndex).toBe(1);
    expect(b!.mediaRef?.id).toBe("second");
    expect(b!.sourceTime).toBeCloseTo(2, 4);
  });

  it("legacy clips with null mediaRefId fall back to primary", () => {
    const s = useEditStore.getState();
    s.setPrimaryMediaRef({
      id: "primary", hash: "primary", kind: "video", path: "/tmp/p.mp4",
      duration: 30, width: 640, height: 480,
    });
    useEditStore.setState({
      clips: [
        { id: "c1", sourceStart: 0, sourceEnd: 30, speed: 1, skipFrames: false, fpsOverride: null },
      ],
    });
    const r = useEditStore.getState().resolveAtOutputTime(15);
    expect(r?.mediaRef?.id).toBe("primary");
    expect(r?.sourceTime).toBeCloseTo(15, 4);
  });

  it("resolveAtOutputTime clamps past-end to the last clip, not null", () => {
    // Covers the rAF drift case: playhead slightly overshoots total duration;
    // engine should still get a valid resolution back rather than no-op.
    const s = useEditStore.getState();
    s.initFromVideo(30);
    s.setPrimaryMediaRef({
      id: "primary", hash: "p", kind: "video", path: "/tmp/p.mp4",
      duration: 30, width: 640, height: 480,
    });
    const r = useEditStore.getState().resolveAtOutputTime(30.0001);
    expect(r).not.toBeNull();
    expect(r!.clipIndex).toBe(0);
  });

  it("resolveAtOutputTime returns null on an empty timeline", () => {
    const r = useEditStore.getState().resolveAtOutputTime(0);
    expect(r).toBeNull();
  });

  it("preserves mediaRefId across splitAt and freeze insertions", () => {
    const s = useEditStore.getState();
    s.setPrimaryMediaRef({
      id: "primary", hash: "p", kind: "video", path: "/tmp/p.mp4",
      duration: 60, width: 640, height: 480,
    });
    s.initFromVideo(60);
    // First clip should now carry the primary mediaRefId.
    expect(useEditStore.getState().clips[0].mediaRefId).toBe(PRIMARY_MEDIA_REF_ID);

    s.splitAt(30);
    const after = useEditStore.getState().clips;
    expect(after).toHaveLength(2);
    expect(after[0].mediaRefId).toBe(PRIMARY_MEDIA_REF_ID);
    expect(after[1].mediaRefId).toBe(PRIMARY_MEDIA_REF_ID);

    s.insertFreezeFrame(15, 2);
    const withFreeze = useEditStore.getState().clips;
    const freeze = withFreeze.find((c) => c.type === "freeze");
    expect(freeze?.mediaRefId).toBe(PRIMARY_MEDIA_REF_ID);
  });
});

describe("image clip support", () => {
  beforeEach(() => useEditStore.getState().reset());

  it("addImageClip appends a clip with the given duration", () => {
    const s = useEditStore.getState();
    s.registerMedia({
      id: "img1", hash: "img1", kind: "image", path: "/tmp/a.png",
      duration: 0, width: 800, height: 600,
    });
    s.addImageClip("img1", 5);
    const clip = useEditStore.getState().clips[0];
    expect(clip.type).toBe("image");
    expect(clip.imageDuration).toBe(5);
    expect(clip.mediaRefId).toBe("img1");
  });

  it("clip output duration comes from imageDuration, not sourceStart/End", () => {
    const s = useEditStore.getState();
    s.registerMedia({
      id: "img1", hash: "img1", kind: "image", path: "/tmp/a.png",
      duration: 0, width: 800, height: 600,
    });
    s.addImageClip("img1", 4);
    const r = useEditStore.getState().resolveAtOutputTime(2);
    expect(r?.clipOutputDuration).toBe(4);
    // sourceTime is meaningless for images — should be 0.
    expect(r?.sourceTime).toBe(0);
  });

  it("setImageDuration updates the output span and affects total", () => {
    const s = useEditStore.getState();
    s.registerMedia({
      id: "img1", hash: "img1", kind: "image", path: "/tmp/a.png",
      duration: 0, width: 800, height: 600,
    });
    s.addImageClip("img1", 3);
    s.setImageDuration(0, 7);
    expect(useEditStore.getState().clips[0].imageDuration).toBe(7);
    expect(useEditStore.getState().getOutputDuration()).toBe(7);
  });

  it("setImageDuration ignores non-image clips", () => {
    const s = useEditStore.getState();
    s.initFromVideo(60);
    s.setImageDuration(0, 10);
    // Should not have changed anything about a normal clip
    expect(useEditStore.getState().clips[0].imageDuration).toBeUndefined();
  });
});

describe("media pool", () => {
  beforeEach(() => useEditStore.getState().reset());

  it("registerMedia upserts by id", () => {
    const s = useEditStore.getState();
    s.registerMedia({ id: "a", hash: "a", kind: "video", path: "/x.mp4", duration: 10, width: 1, height: 1 });
    s.registerMedia({ id: "a", hash: "a", kind: "video", path: "/x.mp4", duration: 20, width: 1, height: 1 });
    expect(useEditStore.getState().mediaPool.a.duration).toBe(20);
  });

  it("setPrimaryMediaRef normalises id to PRIMARY_MEDIA_REF_ID", () => {
    const s = useEditStore.getState();
    s.setPrimaryMediaRef({
      id: "some-other-id", hash: "h", kind: "video", path: "/p.mp4",
      duration: 60, width: 1920, height: 1080,
    });
    expect(useEditStore.getState().primaryMediaRefId).toBe(PRIMARY_MEDIA_REF_ID);
    expect(useEditStore.getState().mediaPool[PRIMARY_MEDIA_REF_ID].path).toBe("/p.mp4");
  });

  it("resolveClipMedia follows null mediaRefId → primary", () => {
    const s = useEditStore.getState();
    s.setPrimaryMediaRef({
      id: "primary", hash: "p", kind: "video", path: "/p.mp4",
      duration: 60, width: 1, height: 1,
    });
    const legacyClip = { id: "c", sourceStart: 0, sourceEnd: 10, speed: 1, skipFrames: false, fpsOverride: null };
    expect(s.resolveClipMedia(legacyClip)?.id).toBe(PRIMARY_MEDIA_REF_ID);
  });

  it("reset wipes the media pool", () => {
    const s = useEditStore.getState();
    s.registerMedia({ id: "a", hash: "a", kind: "video", path: "/x", duration: 1, width: 1, height: 1 });
    s.reset();
    expect(Object.keys(useEditStore.getState().mediaPool)).toHaveLength(0);
    expect(useEditStore.getState().primaryMediaRefId).toBeNull();
  });
});
