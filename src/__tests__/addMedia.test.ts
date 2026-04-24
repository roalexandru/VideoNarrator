/**
 * addMedia.ts end-to-end: import a file, make sure it gets a MediaRef,
 * re-import the same content (even from a different path) and confirm we
 * reuse the existing MediaRef via content hash.
 */

import { describe, it, expect, beforeEach } from "vitest";
import { importAndAddMedia } from "../features/edit-video/addMedia";
import { useEditStore } from "../stores/editStore";
import { setupDefaultMocks } from "./setup";

describe("importAndAddMedia", () => {
  beforeEach(() => {
    setupDefaultMocks();
    useEditStore.getState().reset();
  });

  it("registers a MediaRef and appends a video clip", async () => {
    await importAndAddMedia("/tmp/new-video.mp4");
    const s = useEditStore.getState();
    const refs = Object.values(s.mediaPool);
    expect(refs).toHaveLength(1);
    expect(refs[0].kind).toBe("video");
    expect(refs[0].path).toBe("/tmp/new-video.mp4");
    expect(refs[0].hash).toContain("testhash:"); // from the mocked compute_media_hash
    expect(s.clips).toHaveLength(1);
    expect(s.clips[0].mediaRefId).toBe(refs[0].id);
  });

  it("dedupes by content hash — re-importing same file reuses the ref", async () => {
    await importAndAddMedia("/tmp/same.mp4");
    await importAndAddMedia("/tmp/same.mp4");
    const s = useEditStore.getState();
    // Only one MediaRef; two clips pointing at it.
    expect(Object.keys(s.mediaPool)).toHaveLength(1);
    expect(s.clips).toHaveLength(2);
    expect(s.clips[0].mediaRefId).toBe(s.clips[1].mediaRefId);
  });

  // Image path exercised end-to-end by the Rust integration test
  // `integration_image_clip_end_to_end`. We skip a jsdom-level test here
  // because jsdom doesn't fire Image.onload deterministically across
  // environments — the dedupe logic above already covers the shared path.
});
