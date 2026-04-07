import { describe, it, expect, beforeEach } from "vitest";
import { setupDefaultMocks, resetAllStores } from "./setup";
import { useScriptStore } from "../stores/scriptStore";
import type { NarrationScript, Segment } from "../types/script";

beforeEach(() => {
  setupDefaultMocks();
  resetAllStores();
});

function makeSeg(overrides: Partial<Segment> = {}): Segment {
  return {
    index: 0,
    start_seconds: 0,
    end_seconds: 10,
    text: "Hello world this is a test",
    visual_description: "Title screen",
    emphasis: [],
    pace: "medium",
    pause_after_ms: 0,
    frame_refs: [0],
    ...overrides,
  };
}

function makeScript(segments: Segment[]): NarrationScript {
  return {
    title: "Test Script",
    total_duration_seconds: 60,
    segments,
    metadata: {
      style: "product_demo",
      language: "en",
      provider: "claude",
      model: "claude-sonnet-4-20250514",
      generated_at: "2026-04-03T14:00:00Z",
    },
  };
}

describe("setScript", () => {
  it("stores segments correctly", () => {
    const script = makeScript([
      makeSeg({ index: 0, text: "First", start_seconds: 0, end_seconds: 10 }),
      makeSeg({ index: 1, text: "Second", start_seconds: 10, end_seconds: 20 }),
    ]);
    useScriptStore.getState().setScript("en", script);

    const stored = useScriptStore.getState().scripts["en"];
    expect(stored).toBeDefined();
    expect(stored.segments).toHaveLength(2);
    expect(stored.segments[0].text).toBe("First");
    expect(stored.segments[1].text).toBe("Second");
    expect(stored.title).toBe("Test Script");
  });

  it("stores metadata correctly", () => {
    const script = makeScript([makeSeg()]);
    useScriptStore.getState().setScript("en", script);

    const meta = useScriptStore.getState().scripts["en"].metadata;
    expect(meta.style).toBe("product_demo");
    expect(meta.language).toBe("en");
    expect(meta.provider).toBe("claude");
    expect(meta.model).toBe("claude-sonnet-4-20250514");
  });
});

describe("setScripts with multiple languages", () => {
  it("stores scripts for multiple languages independently", () => {
    const enScript = makeScript([
      makeSeg({ index: 0, text: "Hello world" }),
    ]);
    const jaScript = makeScript([
      makeSeg({ index: 0, text: "こんにちは世界" }),
    ]);
    jaScript.metadata = { ...jaScript.metadata, language: "ja" };

    useScriptStore.getState().setScript("en", enScript);
    useScriptStore.getState().setScript("ja", jaScript);

    const scripts = useScriptStore.getState().scripts;
    expect(Object.keys(scripts)).toHaveLength(2);
    expect(scripts["en"].segments[0].text).toBe("Hello world");
    expect(scripts["ja"].segments[0].text).toBe("こんにちは世界");
  });

  it("overwriting a language replaces the script", () => {
    const first = makeScript([makeSeg({ text: "Original" })]);
    const second = makeScript([makeSeg({ text: "Updated" })]);

    useScriptStore.getState().setScript("en", first);
    useScriptStore.getState().setScript("en", second);

    expect(useScriptStore.getState().scripts["en"].segments[0].text).toBe("Updated");
  });
});

describe("updateSegmentText", () => {
  it("changes text and preserves other fields", () => {
    const script = makeScript([
      makeSeg({
        index: 0,
        text: "Original text",
        start_seconds: 5,
        end_seconds: 15,
        visual_description: "Intro scene",
        emphasis: ["important"],
        pace: "slow",
        pause_after_ms: 500,
        frame_refs: [0, 1, 2],
      }),
    ]);
    useScriptStore.getState().setScript("en", script);
    useScriptStore.getState().updateSegmentText("en", 0, "Modified text");

    const seg = useScriptStore.getState().scripts["en"].segments[0];
    expect(seg.text).toBe("Modified text");
    // All other fields preserved
    expect(seg.start_seconds).toBe(5);
    expect(seg.end_seconds).toBe(15);
    expect(seg.visual_description).toBe("Intro scene");
    expect(seg.emphasis).toEqual(["important"]);
    expect(seg.pace).toBe("slow");
    expect(seg.pause_after_ms).toBe(500);
    expect(seg.frame_refs).toEqual([0, 1, 2]);
  });

  it("does nothing for non-existent language", () => {
    useScriptStore.getState().updateSegmentText("fr", 0, "Bonjour");
    expect(useScriptStore.getState().scripts["fr"]).toBeUndefined();
  });
});

describe("deleteSegment", () => {
  it("removes and re-indexes", () => {
    const script = makeScript([
      makeSeg({ index: 0, text: "First", start_seconds: 0, end_seconds: 10 }),
      makeSeg({ index: 1, text: "Second", start_seconds: 10, end_seconds: 20 }),
      makeSeg({ index: 2, text: "Third", start_seconds: 20, end_seconds: 30 }),
    ]);
    useScriptStore.getState().setScript("en", script);
    useScriptStore.getState().deleteSegment("en", 1);

    const segs = useScriptStore.getState().scripts["en"].segments;
    expect(segs).toHaveLength(2);
    expect(segs[0].text).toBe("First");
    expect(segs[0].index).toBe(0);
    expect(segs[1].text).toBe("Third");
    expect(segs[1].index).toBe(1); // re-indexed
  });

  it("deleting first segment re-indexes remaining", () => {
    const script = makeScript([
      makeSeg({ index: 0, text: "A" }),
      makeSeg({ index: 1, text: "B" }),
    ]);
    useScriptStore.getState().setScript("en", script);
    useScriptStore.getState().deleteSegment("en", 0);

    const segs = useScriptStore.getState().scripts["en"].segments;
    expect(segs).toHaveLength(1);
    expect(segs[0].text).toBe("B");
    expect(segs[0].index).toBe(0);
  });

  it("deleting last segment re-indexes (no-op since single remaining)", () => {
    const script = makeScript([
      makeSeg({ index: 0, text: "A" }),
      makeSeg({ index: 1, text: "B" }),
    ]);
    useScriptStore.getState().setScript("en", script);
    useScriptStore.getState().deleteSegment("en", 1);

    const segs = useScriptStore.getState().scripts["en"].segments;
    expect(segs).toHaveLength(1);
    expect(segs[0].text).toBe("A");
    expect(segs[0].index).toBe(0);
  });
});

describe("splitSegment", () => {
  it("creates two segments with correct timing", () => {
    const script = makeScript([
      makeSeg({ index: 0, start_seconds: 0, end_seconds: 20, text: "Hello beautiful world" }),
    ]);
    useScriptStore.getState().setScript("en", script);
    useScriptStore.getState().splitSegment("en", 0, 10);

    const segs = useScriptStore.getState().scripts["en"].segments;
    expect(segs).toHaveLength(2);
    expect(segs[0].start_seconds).toBe(0);
    expect(segs[0].end_seconds).toBe(10);
    expect(segs[1].start_seconds).toBe(10);
    expect(segs[1].end_seconds).toBe(20);
    // Both segments re-indexed
    expect(segs[0].index).toBe(0);
    expect(segs[1].index).toBe(1);
  });

  it("text is split at word boundary (FE-4 fix verification)", () => {
    const text = "Hello beautiful world of testing today";
    const script = makeScript([
      makeSeg({ index: 0, start_seconds: 0, end_seconds: 20, text }),
    ]);
    useScriptStore.getState().setScript("en", script);
    useScriptStore.getState().splitSegment("en", 0, 10);

    const segs = useScriptStore.getState().scripts["en"].segments;
    expect(segs).toHaveLength(2);
    // Both texts should be trimmed (no leading/trailing spaces)
    expect(segs[0].text).not.toMatch(/^\s/);
    expect(segs[0].text).not.toMatch(/\s$/);
    expect(segs[1].text).not.toMatch(/^\s/);
    expect(segs[1].text).not.toMatch(/\s$/);
    // Combined text should reconstruct original
    const combined = segs[0].text + " " + segs[1].text;
    expect(combined).toBe(text);
  });

  it("does not split if splitAtSeconds is at start boundary", () => {
    const script = makeScript([
      makeSeg({ index: 0, start_seconds: 5, end_seconds: 15, text: "Hello world" }),
    ]);
    useScriptStore.getState().setScript("en", script);
    useScriptStore.getState().splitSegment("en", 0, 5); // at start

    expect(useScriptStore.getState().scripts["en"].segments).toHaveLength(1);
  });

  it("does not split if splitAtSeconds is at end boundary", () => {
    const script = makeScript([
      makeSeg({ index: 0, start_seconds: 5, end_seconds: 15, text: "Hello world" }),
    ]);
    useScriptStore.getState().setScript("en", script);
    useScriptStore.getState().splitSegment("en", 0, 15); // at end

    expect(useScriptStore.getState().scripts["en"].segments).toHaveLength(1);
  });

  it("split in a multi-segment script re-indexes all segments", () => {
    const script = makeScript([
      makeSeg({ index: 0, text: "First part", start_seconds: 0, end_seconds: 10 }),
      makeSeg({ index: 1, text: "Second part here", start_seconds: 10, end_seconds: 20 }),
      makeSeg({ index: 2, text: "Third part", start_seconds: 20, end_seconds: 30 }),
    ]);
    useScriptStore.getState().setScript("en", script);
    useScriptStore.getState().splitSegment("en", 1, 15);

    const segs = useScriptStore.getState().scripts["en"].segments;
    expect(segs).toHaveLength(4);
    expect(segs[0].index).toBe(0);
    expect(segs[1].index).toBe(1);
    expect(segs[2].index).toBe(2);
    expect(segs[3].index).toBe(3);
    expect(segs[0].text).toBe("First part");
    expect(segs[3].text).toBe("Third part");
  });
});

describe("mergeSegments", () => {
  it("combines text and timing", () => {
    const script = makeScript([
      makeSeg({ index: 0, text: "Hello", start_seconds: 0, end_seconds: 10, frame_refs: [0] }),
      makeSeg({ index: 1, text: "world", start_seconds: 10, end_seconds: 20, frame_refs: [1] }),
    ]);
    useScriptStore.getState().setScript("en", script);
    useScriptStore.getState().mergeSegments("en", 0, 1);

    const segs = useScriptStore.getState().scripts["en"].segments;
    expect(segs).toHaveLength(1);
    expect(segs[0].text).toBe("Hello world");
    expect(segs[0].start_seconds).toBe(0);
    expect(segs[0].end_seconds).toBe(20);
    expect(segs[0].frame_refs).toEqual([0, 1]);
    expect(segs[0].index).toBe(0);
  });

  it("merging three segments works", () => {
    const script = makeScript([
      makeSeg({ index: 0, text: "A", start_seconds: 0, end_seconds: 5, frame_refs: [0] }),
      makeSeg({ index: 1, text: "B", start_seconds: 5, end_seconds: 10, frame_refs: [1] }),
      makeSeg({ index: 2, text: "C", start_seconds: 10, end_seconds: 15, frame_refs: [2] }),
    ]);
    useScriptStore.getState().setScript("en", script);
    useScriptStore.getState().mergeSegments("en", 0, 2);

    const segs = useScriptStore.getState().scripts["en"].segments;
    expect(segs).toHaveLength(1);
    expect(segs[0].text).toBe("A B C");
    expect(segs[0].start_seconds).toBe(0);
    expect(segs[0].end_seconds).toBe(15);
    expect(segs[0].frame_refs).toEqual([0, 1, 2]);
  });

  it("merge does nothing for a single segment range", () => {
    const script = makeScript([
      makeSeg({ index: 0, text: "Only one" }),
    ]);
    useScriptStore.getState().setScript("en", script);
    useScriptStore.getState().mergeSegments("en", 0, 0);

    expect(useScriptStore.getState().scripts["en"].segments).toHaveLength(1);
    expect(useScriptStore.getState().scripts["en"].segments[0].text).toBe("Only one");
  });

  it("merge re-indexes remaining segments", () => {
    const script = makeScript([
      makeSeg({ index: 0, text: "A", start_seconds: 0, end_seconds: 5, frame_refs: [0] }),
      makeSeg({ index: 1, text: "B", start_seconds: 5, end_seconds: 10, frame_refs: [1] }),
      makeSeg({ index: 2, text: "C", start_seconds: 10, end_seconds: 15, frame_refs: [2] }),
      makeSeg({ index: 3, text: "D", start_seconds: 15, end_seconds: 20, frame_refs: [3] }),
    ]);
    useScriptStore.getState().setScript("en", script);
    useScriptStore.getState().mergeSegments("en", 1, 2);

    const segs = useScriptStore.getState().scripts["en"].segments;
    expect(segs).toHaveLength(3);
    expect(segs[0].index).toBe(0);
    expect(segs[0].text).toBe("A");
    expect(segs[1].index).toBe(1);
    expect(segs[1].text).toBe("B C");
    expect(segs[2].index).toBe(2);
    expect(segs[2].text).toBe("D");
  });
});

describe("setActiveLanguage", () => {
  it("switches active language", () => {
    useScriptStore.getState().setActiveLanguage("ja");
    expect(useScriptStore.getState().activeLanguage).toBe("ja");
  });

  it("default active language is en", () => {
    expect(useScriptStore.getState().activeLanguage).toBe("en");
  });
});

describe("getActiveScript (via activeLanguage)", () => {
  it("returns correct language script when activeLanguage is set", () => {
    const enScript = makeScript([makeSeg({ text: "English text" })]);
    const jaScript = makeScript([makeSeg({ text: "日本語テキスト" })]);

    useScriptStore.getState().setScript("en", enScript);
    useScriptStore.getState().setScript("ja", jaScript);

    useScriptStore.getState().setActiveLanguage("en");
    const enActive = useScriptStore.getState().scripts[useScriptStore.getState().activeLanguage];
    expect(enActive.segments[0].text).toBe("English text");

    useScriptStore.getState().setActiveLanguage("ja");
    const jaActive = useScriptStore.getState().scripts[useScriptStore.getState().activeLanguage];
    expect(jaActive.segments[0].text).toBe("日本語テキスト");
  });
});

describe("setActiveSegment", () => {
  it("sets and clears active segment index", () => {
    useScriptStore.getState().setActiveSegment(2);
    expect(useScriptStore.getState().activeSegmentIndex).toBe(2);

    useScriptStore.getState().setActiveSegment(null);
    expect(useScriptStore.getState().activeSegmentIndex).toBeNull();
  });
});

describe("reset", () => {
  it("clears all scripts and resets defaults", () => {
    useScriptStore.getState().setScript("en", makeScript([makeSeg()]));
    useScriptStore.getState().setScript("ja", makeScript([makeSeg()]));
    useScriptStore.getState().setActiveLanguage("ja");
    useScriptStore.getState().setActiveSegment(5);

    useScriptStore.getState().reset();

    const state = useScriptStore.getState();
    expect(state.scripts).toEqual({});
    expect(state.activeLanguage).toBe("en");
    expect(state.activeSegmentIndex).toBeNull();
  });
});
