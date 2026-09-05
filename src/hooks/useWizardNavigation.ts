import { create } from "zustand";
import { trackEvent } from "../features/telemetry/analytics";

const STEP_LABELS = [
  "Project Setup",
  "Edit Video",
  "Configuration",
  "Processing",
  "Review",
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
    set((state) => {
      const next = Math.min(state.currentStep + 1, 5);
      // Next at the last step clamps to where we already are. Tracking that
      // reports a navigation that never happened, and `step_visited` is the
      // single most common event — phantoms distort the whole funnel. Same
      // guard `goToStep` has always had.
      if (next === state.currentStep) return state;
      trackEvent("step_visited", { step: STEP_LABELS[next] || `step_${next}`, step_index: next });
      return {
        currentStep: next,
        completedSteps: new Set([...state.completedSteps, state.currentStep]),
      };
    }),

  goBack: () =>
    set((state) => {
      const prev = Math.max(state.currentStep - 1, 0);
      // Back at the first step clamps to itself — see `goNext`.
      if (prev === state.currentStep) return state;
      trackEvent("step_visited", { step: STEP_LABELS[prev] || `step_${prev}`, step_index: prev });
      return {
        currentStep: prev,
      };
    }),

  goToStep: (step) => {
    const clamped = Math.max(0, Math.min(step, 5));
    const current = useWizardStore.getState().currentStep;
    if (clamped === current) return; // Prevent duplicate events from effect re-runs
    trackEvent("step_visited", { step: STEP_LABELS[clamped] || `step_${clamped}`, step_index: clamped });
    set({ currentStep: clamped });
  },

  markCompleted: (step) =>
    set((state) => ({
      completedSteps: new Set([...state.completedSteps, step]),
    })),

  reset: () =>
    set({ currentStep: 0, completedSteps: new Set<number>() }),
}));

export { STEP_LABELS };
