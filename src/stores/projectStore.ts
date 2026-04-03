import { create } from "zustand";
import type { VideoFile, ContextDocument } from "../types/project";

interface ProjectStore {
  projectId: string;
  videoFile: VideoFile | null;
  contextDocuments: ContextDocument[];
  title: string;
  description: string;
  setProjectId: (id: string) => void;
  setVideoFile: (file: VideoFile | null) => void;
  addDocuments: (docs: ContextDocument[]) => void;
  removeDocument: (id: string) => void;
  reorderDocuments: (fromIndex: number, toIndex: number) => void;
  setTitle: (title: string) => void;
  setDescription: (desc: string) => void;
  reset: () => void;
}

export const useProjectStore = create<ProjectStore>((set) => ({
  projectId: "",
  videoFile: null,
  contextDocuments: [],
  title: "",
  description: "",

  setProjectId: (projectId) => set({ projectId }),
  setVideoFile: (file) => set({ videoFile: file }),

  addDocuments: (docs) =>
    set((state) => ({
      contextDocuments: [...state.contextDocuments, ...docs],
    })),

  removeDocument: (id) =>
    set((state) => ({
      contextDocuments: state.contextDocuments.filter((d) => d.id !== id),
    })),

  reorderDocuments: (fromIndex, toIndex) =>
    set((state) => {
      const docs = [...state.contextDocuments];
      const [item] = docs.splice(fromIndex, 1);
      docs.splice(toIndex, 0, item);
      return { contextDocuments: docs };
    }),

  setTitle: (title) => set({ title }),
  setDescription: (description) => set({ description }),

  reset: () =>
    set({
      projectId: "",
      videoFile: null,
      contextDocuments: [],
      title: "",
      description: "",
    }),
}));
