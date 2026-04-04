import { describe, it, expect, beforeEach } from "vitest";
import { useProjectStore } from "../../stores/projectStore";
import type { VideoFile, ContextDocument } from "../../types/project";

function makeVideoFile(overrides: Partial<VideoFile> = {}): VideoFile {
  return {
    path: "/tmp/test.mp4",
    name: "test.mp4",
    size: 50_000_000,
    duration: 120,
    resolution: { width: 1920, height: 1080 },
    codec: "h264",
    fps: 30,
    ...overrides,
  };
}

function makeDocument(overrides: Partial<ContextDocument> = {}): ContextDocument {
  return {
    id: "doc-1",
    path: "/tmp/doc.md",
    name: "doc.md",
    size: 1024,
    type: "md",
    ...overrides,
  };
}

describe("projectStore", () => {
  beforeEach(() => {
    useProjectStore.getState().reset();
  });

  describe("setVideoFile", () => {
    it("stores video metadata", () => {
      const video = makeVideoFile();
      useProjectStore.getState().setVideoFile(video);

      const state = useProjectStore.getState();
      expect(state.videoFile).not.toBeNull();
      expect(state.videoFile!.path).toBe("/tmp/test.mp4");
      expect(state.videoFile!.name).toBe("test.mp4");
      expect(state.videoFile!.duration).toBe(120);
      expect(state.videoFile!.resolution).toEqual({ width: 1920, height: 1080 });
      expect(state.videoFile!.codec).toBe("h264");
      expect(state.videoFile!.fps).toBe(30);
      expect(state.videoFile!.size).toBe(50_000_000);
    });

    it("can be set to null", () => {
      useProjectStore.getState().setVideoFile(makeVideoFile());
      useProjectStore.getState().setVideoFile(null);
      expect(useProjectStore.getState().videoFile).toBeNull();
    });
  });

  describe("setTitle / setDescription", () => {
    it("setTitle updates state", () => {
      useProjectStore.getState().setTitle("My Project");
      expect(useProjectStore.getState().title).toBe("My Project");
    });

    it("setDescription updates state", () => {
      useProjectStore.getState().setDescription("A detailed description");
      expect(useProjectStore.getState().description).toBe("A detailed description");
    });
  });

  describe("addDocuments / removeDocument", () => {
    it("addDocuments appends documents", () => {
      const doc1 = makeDocument({ id: "doc-1", name: "a.md" });
      const doc2 = makeDocument({ id: "doc-2", name: "b.txt", type: "txt" });

      useProjectStore.getState().addDocuments([doc1]);
      expect(useProjectStore.getState().contextDocuments).toHaveLength(1);

      useProjectStore.getState().addDocuments([doc2]);
      expect(useProjectStore.getState().contextDocuments).toHaveLength(2);
      expect(useProjectStore.getState().contextDocuments[0].name).toBe("a.md");
      expect(useProjectStore.getState().contextDocuments[1].name).toBe("b.txt");
    });

    it("addDocuments appends multiple at once", () => {
      const docs = [
        makeDocument({ id: "doc-1" }),
        makeDocument({ id: "doc-2" }),
        makeDocument({ id: "doc-3" }),
      ];
      useProjectStore.getState().addDocuments(docs);
      expect(useProjectStore.getState().contextDocuments).toHaveLength(3);
    });

    it("removeDocument removes by id", () => {
      const docs = [
        makeDocument({ id: "doc-1", name: "first.md" }),
        makeDocument({ id: "doc-2", name: "second.md" }),
        makeDocument({ id: "doc-3", name: "third.md" }),
      ];
      useProjectStore.getState().addDocuments(docs);

      useProjectStore.getState().removeDocument("doc-2");

      const remaining = useProjectStore.getState().contextDocuments;
      expect(remaining).toHaveLength(2);
      expect(remaining.map((d) => d.id)).toEqual(["doc-1", "doc-3"]);
    });

    it("removeDocument does nothing for non-existent id", () => {
      useProjectStore.getState().addDocuments([makeDocument({ id: "doc-1" })]);
      useProjectStore.getState().removeDocument("non-existent");
      expect(useProjectStore.getState().contextDocuments).toHaveLength(1);
    });
  });

  describe("reorderDocuments", () => {
    it("moves a document from one position to another", () => {
      const docs = [
        makeDocument({ id: "a" }),
        makeDocument({ id: "b" }),
        makeDocument({ id: "c" }),
      ];
      useProjectStore.getState().addDocuments(docs);

      useProjectStore.getState().reorderDocuments(0, 2);

      const ids = useProjectStore.getState().contextDocuments.map((d) => d.id);
      expect(ids).toEqual(["b", "c", "a"]);
    });
  });

  describe("setProjectId", () => {
    it("stores the ID", () => {
      useProjectStore.getState().setProjectId("proj-abc-123");
      expect(useProjectStore.getState().projectId).toBe("proj-abc-123");
    });
  });

  describe("reset", () => {
    it("clears all state", () => {
      useProjectStore.getState().setProjectId("proj-1");
      useProjectStore.getState().setVideoFile(makeVideoFile());
      useProjectStore.getState().setTitle("Title");
      useProjectStore.getState().setDescription("Desc");
      useProjectStore.getState().addDocuments([makeDocument()]);

      useProjectStore.getState().reset();

      const state = useProjectStore.getState();
      expect(state.projectId).toBe("");
      expect(state.videoFile).toBeNull();
      expect(state.title).toBe("");
      expect(state.description).toBe("");
      expect(state.contextDocuments).toHaveLength(0);
    });
  });
});
