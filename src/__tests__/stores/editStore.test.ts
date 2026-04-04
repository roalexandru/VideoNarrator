import { describe, it, expect, beforeEach } from "vitest";
import { useEditStore } from "../../stores/editStore";

describe("editStore", () => {
  beforeEach(() => {
    useEditStore.getState().reset();
  });

  describe("initFromVideo", () => {
    it("creates one clip spanning the full duration", () => {
      useEditStore.getState().initFromVideo(60);

      const { clips, sourceDuration, selectedClipIndex } = useEditStore.getState();
      expect(clips).toHaveLength(1);
      expect(clips[0].sourceStart).toBe(0);
      expect(clips[0].sourceEnd).toBe(60);
      expect(clips[0].speed).toBe(1.0);
      expect(clips[0].skipFrames).toBe(false);
      expect(clips[0].fpsOverride).toBeNull();
      expect(sourceDuration).toBe(60);
      expect(selectedClipIndex).toBe(0);
    });
  });

  describe("splitAt", () => {
    it("splits a clip into two at the correct source positions", () => {
      useEditStore.getState().initFromVideo(60);

      // Split at output time 20s (with speed 1.0, source time = output time)
      useEditStore.getState().splitAt(20);

      const { clips } = useEditStore.getState();
      expect(clips).toHaveLength(2);
      expect(clips[0].sourceStart).toBe(0);
      expect(clips[0].sourceEnd).toBe(20);
      expect(clips[1].sourceStart).toBe(20);
      expect(clips[1].sourceEnd).toBe(60);
    });

    it("splits correctly when clip has a non-1.0 speed", () => {
      useEditStore.getState().initFromVideo(60);
      useEditStore.getState().setClipSpeed(0, 2.0);

      // With speed 2.0, the clip output duration is 60/2 = 30s.
      // Splitting at output time 10s means source time offset = 10 * 2 = 20s.
      useEditStore.getState().splitAt(10);

      const { clips } = useEditStore.getState();
      expect(clips).toHaveLength(2);
      expect(clips[0].sourceStart).toBe(0);
      expect(clips[0].sourceEnd).toBe(20);
      expect(clips[1].sourceStart).toBe(20);
      expect(clips[1].sourceEnd).toBe(60);
      // Both inherit the parent speed
      expect(clips[0].speed).toBe(2.0);
      expect(clips[1].speed).toBe(2.0);
    });

    it("does nothing when split time is at the boundary", () => {
      useEditStore.getState().initFromVideo(60);
      useEditStore.getState().splitAt(0); // at the very start
      expect(useEditStore.getState().clips).toHaveLength(1);

      useEditStore.getState().splitAt(60); // at the very end
      expect(useEditStore.getState().clips).toHaveLength(1);
    });
  });

  describe("deleteClip", () => {
    it("removes a clip and remaining clips collapse (no gaps)", () => {
      useEditStore.getState().initFromVideo(60);
      useEditStore.getState().splitAt(20); // [0-20], [20-60]
      useEditStore.getState().splitAt(40); // [0-20], [20-40], [40-60] (split second clip at output 40; first clip takes 20s output, second starts at 20)

      expect(useEditStore.getState().clips).toHaveLength(3);

      // Delete the middle clip
      useEditStore.getState().deleteClip(1);

      const { clips } = useEditStore.getState();
      expect(clips).toHaveLength(2);
      // The remaining clips are the first and last
      expect(clips[0].sourceStart).toBe(0);
      expect(clips[0].sourceEnd).toBe(20);
      expect(clips[1].sourceStart).toBe(40);
      expect(clips[1].sourceEnd).toBe(60);
    });

    it("does not delete the last remaining clip", () => {
      useEditStore.getState().initFromVideo(60);
      useEditStore.getState().deleteClip(0);

      expect(useEditStore.getState().clips).toHaveLength(1);
    });

    it("adjusts selectedClipIndex when deleting last clip in array", () => {
      useEditStore.getState().initFromVideo(60);
      useEditStore.getState().splitAt(30);
      expect(useEditStore.getState().clips).toHaveLength(2);

      useEditStore.getState().deleteClip(1);
      expect(useEditStore.getState().selectedClipIndex).toBe(0);
    });
  });

  describe("moveClip", () => {
    it("reorders clips", () => {
      useEditStore.getState().initFromVideo(90);
      useEditStore.getState().splitAt(30); // [0-30], [30-90]
      useEditStore.getState().splitAt(60); // [0-30], [30-60], [60-90]

      const idsBefore = useEditStore.getState().clips.map((c) => c.id);

      useEditStore.getState().moveClip(0, 2);

      const clips = useEditStore.getState().clips;
      expect(clips).toHaveLength(3);
      // First clip moved to position 2
      expect(clips[0].id).toBe(idsBefore[1]);
      expect(clips[1].id).toBe(idsBefore[2]);
      expect(clips[2].id).toBe(idsBefore[0]);
      expect(useEditStore.getState().selectedClipIndex).toBe(2);
    });

    it("does nothing when fromIndex equals toIndex", () => {
      useEditStore.getState().initFromVideo(60);
      useEditStore.getState().splitAt(30);
      const clipsBefore = [...useEditStore.getState().clips];

      useEditStore.getState().moveClip(0, 0);

      expect(useEditStore.getState().clips.map((c) => c.id)).toEqual(
        clipsBefore.map((c) => c.id)
      );
    });
  });

  describe("setClipSpeed", () => {
    it("changes speed and affects getOutputDuration", () => {
      useEditStore.getState().initFromVideo(60);
      expect(useEditStore.getState().getOutputDuration()).toBe(60);

      useEditStore.getState().setClipSpeed(0, 2.0);
      expect(useEditStore.getState().getOutputDuration()).toBe(30);
    });
  });

  describe("getOutputDuration", () => {
    it("calculates total correctly with mixed speeds", () => {
      useEditStore.getState().initFromVideo(100);
      useEditStore.getState().splitAt(50); // [0-50] @1x, [50-100] @1x

      // Set different speeds
      useEditStore.getState().setClipSpeed(0, 2.0); // 50/2 = 25s output
      useEditStore.getState().setClipSpeed(1, 0.5); // 50/0.5 = 100s output

      expect(useEditStore.getState().getOutputDuration()).toBe(125);
    });

    it("returns 0 for empty clips", () => {
      expect(useEditStore.getState().getOutputDuration()).toBe(0);
    });
  });

  describe("outputTimeToSource", () => {
    it("maps output time back to correct source position with uniform speed", () => {
      useEditStore.getState().initFromVideo(60);
      expect(useEditStore.getState().outputTimeToSource(30)).toBe(30);
    });

    it("maps output time back correctly with mixed speeds", () => {
      useEditStore.getState().initFromVideo(100);
      useEditStore.getState().splitAt(50); // [0-50] @1x, [50-100] @1x
      useEditStore.getState().setClipSpeed(0, 2.0); // output duration = 25s
      useEditStore.getState().setClipSpeed(1, 1.0); // output duration = 50s

      // Time 0 -> source 0
      expect(useEditStore.getState().outputTimeToSource(0)).toBe(0);

      // Time 12.5 -> in first clip: 12.5 * 2.0 = 25 source seconds -> source 25
      expect(useEditStore.getState().outputTimeToSource(12.5)).toBe(25);

      // Time 25 -> boundary of first clip (output dur = 25), should be end of first clip
      // At cumulative=25, we enter second clip. relativeOutput = 0, source = 50 + 0*1 = 50
      expect(useEditStore.getState().outputTimeToSource(25)).toBe(50);

      // Time 35 -> in second clip: 35 - 25 = 10 into second clip, source = 50 + 10*1 = 60
      expect(useEditStore.getState().outputTimeToSource(35)).toBe(60);
    });

    it("returns last clip end when past total output duration", () => {
      useEditStore.getState().initFromVideo(60);
      expect(useEditStore.getState().outputTimeToSource(999)).toBe(60);
    });

    it("returns 0 for empty clips", () => {
      expect(useEditStore.getState().outputTimeToSource(10)).toBe(0);
    });
  });

  describe("reset", () => {
    it("clears everything", () => {
      useEditStore.getState().initFromVideo(60);
      useEditStore.getState().splitAt(30);
      useEditStore.getState().setEditedVideoPath("/tmp/edited.mp4");

      useEditStore.getState().reset();

      const state = useEditStore.getState();
      expect(state.clips).toHaveLength(0);
      expect(state.selectedClipIndex).toBeNull();
      expect(state.editedVideoPath).toBeNull();
      expect(state.sourceDuration).toBe(0);
    });
  });

  describe("getClipOutputStart", () => {
    it("returns cumulative output time before a given clip index", () => {
      useEditStore.getState().initFromVideo(100);
      useEditStore.getState().splitAt(40); // [0-40], [40-100]
      useEditStore.getState().setClipSpeed(0, 2.0); // output = 20s

      expect(useEditStore.getState().getClipOutputStart(0)).toBe(0);
      expect(useEditStore.getState().getClipOutputStart(1)).toBe(20); // 40/2 = 20
    });
  });

  describe("addClip", () => {
    it("appends a new clip at the end", () => {
      useEditStore.getState().initFromVideo(60);
      useEditStore.getState().addClip("source.mp4", 10, 20);

      const { clips, selectedClipIndex } = useEditStore.getState();
      expect(clips).toHaveLength(2);
      expect(clips[1].sourceStart).toBe(10);
      expect(clips[1].sourceEnd).toBe(20);
      expect(clips[1].speed).toBe(1.0);
      expect(selectedClipIndex).toBe(1);
    });
  });

  describe("selectClip", () => {
    it("sets the selected clip index", () => {
      useEditStore.getState().initFromVideo(60);
      useEditStore.getState().splitAt(30);
      useEditStore.getState().selectClip(1);
      expect(useEditStore.getState().selectedClipIndex).toBe(1);

      useEditStore.getState().selectClip(null);
      expect(useEditStore.getState().selectedClipIndex).toBeNull();
    });
  });
});
