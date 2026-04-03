import { create } from "zustand";

const STEP_LABELS = [
  "Project Setup",
  "Edit Video",
  "Configuration",
  "Processing",
  "Review & Edit",
  "Export",
] as const;

interface WizardState {
  currentStep: number;
  completedSteps: Set<number>;
  goNext: () => void;
  goBack: () => void;
  goToStep: (step: number) => void;
  markCompleted: (step: number) => void;
  reset: () => void;
}

export const useWizardStore = create<WizardState>((set) => ({
  currentStep: 0,
  completedSteps: new Set<number>(),

  goNext: () =>
    set((state) => ({
      currentStep: Math.min(state.currentStep + 1, 5),
      completedSteps: new Set([...state.completedSteps, state.currentStep]),
    })),

  goBack: () =>
    set((state) => ({
      currentStep: Math.max(state.currentStep - 1, 0),
    })),

  goToStep: (step) =>
    set({ currentStep: Math.max(0, Math.min(step, 5)) }),

  markCompleted: (step) =>
    set((state) => ({
      completedSteps: new Set([...state.completedSteps, step]),
    })),

  reset: () =>
    set({ currentStep: 0, completedSteps: new Set<number>() }),
}));

export { STEP_LABELS };
