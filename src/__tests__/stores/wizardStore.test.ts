import { describe, it, expect, beforeEach } from "vitest";
import { useWizardStore, STEP_LABELS } from "../../hooks/useWizardNavigation";

describe("wizardStore", () => {
  beforeEach(() => {
    useWizardStore.getState().reset();
  });

  it("has correct initial state", () => {
    const state = useWizardStore.getState();
    expect(state.currentStep).toBe(0);
    expect(state.completedSteps.size).toBe(0);
  });

  it("exports STEP_LABELS with 6 entries", () => {
    expect(STEP_LABELS).toHaveLength(6);
    expect(STEP_LABELS[0]).toBe("Project Setup");
    expect(STEP_LABELS[5]).toBe("Export");
  });

  it("goNext increments step", () => {
    useWizardStore.getState().goNext();
    expect(useWizardStore.getState().currentStep).toBe(1);
  });

  it("goNext marks the previous step as completed", () => {
    useWizardStore.getState().goNext();
    expect(useWizardStore.getState().completedSteps.has(0)).toBe(true);
  });

  it("goNext does not exceed the maximum step (5)", () => {
    for (let i = 0; i < 10; i++) {
      useWizardStore.getState().goNext();
    }
    expect(useWizardStore.getState().currentStep).toBe(5);
  });

  it("goBack decrements step", () => {
    useWizardStore.getState().goNext();
    useWizardStore.getState().goNext();
    expect(useWizardStore.getState().currentStep).toBe(2);

    useWizardStore.getState().goBack();
    expect(useWizardStore.getState().currentStep).toBe(1);
  });

  it("cannot go below step 0", () => {
    useWizardStore.getState().goBack();
    expect(useWizardStore.getState().currentStep).toBe(0);

    useWizardStore.getState().goBack();
    useWizardStore.getState().goBack();
    expect(useWizardStore.getState().currentStep).toBe(0);
  });

  it("goToStep sets step directly", () => {
    useWizardStore.getState().goToStep(3);
    expect(useWizardStore.getState().currentStep).toBe(3);
  });

  it("goToStep clamps to valid range", () => {
    useWizardStore.getState().goToStep(-5);
    expect(useWizardStore.getState().currentStep).toBe(0);

    useWizardStore.getState().goToStep(99);
    expect(useWizardStore.getState().currentStep).toBe(5);
  });

  it("markCompleted adds step to completedSteps", () => {
    useWizardStore.getState().markCompleted(2);
    expect(useWizardStore.getState().completedSteps.has(2)).toBe(true);

    useWizardStore.getState().markCompleted(4);
    expect(useWizardStore.getState().completedSteps.has(4)).toBe(true);
    expect(useWizardStore.getState().completedSteps.size).toBe(2);
  });

  it("markCompleted is idempotent", () => {
    useWizardStore.getState().markCompleted(1);
    useWizardStore.getState().markCompleted(1);
    expect(useWizardStore.getState().completedSteps.size).toBe(1);
  });

  it("reset returns to step 0 and clears completed steps", () => {
    useWizardStore.getState().goNext();
    useWizardStore.getState().goNext();
    useWizardStore.getState().markCompleted(3);

    useWizardStore.getState().reset();

    const state = useWizardStore.getState();
    expect(state.currentStep).toBe(0);
    expect(state.completedSteps.size).toBe(0);
  });

  it("goNext then goBack preserves completed steps", () => {
    useWizardStore.getState().goNext(); // 0 -> 1, marks 0 completed
    useWizardStore.getState().goNext(); // 1 -> 2, marks 1 completed
    useWizardStore.getState().goBack(); // 2 -> 1

    expect(useWizardStore.getState().currentStep).toBe(1);
    expect(useWizardStore.getState().completedSteps.has(0)).toBe(true);
    expect(useWizardStore.getState().completedSteps.has(1)).toBe(true);
  });
});
