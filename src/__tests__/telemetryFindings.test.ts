/**
 * Guards for the telemetry-instrumentation defects found by auditing a
 * production Aptabase export (100 events, 3 users, 4 sessions).
 *
 * Each test here failed before its fix — these are regression guards for
 * specific observed wrong data, not speculative coverage.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";

const tracked: Array<{ name: string; props?: Record<string, unknown> }> = [];

vi.mock("../features/telemetry/analytics", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../features/telemetry/analytics")>();
  return {
    ...actual,
    trackEvent: (name: string, props?: Record<string, unknown>) => {
      tracked.push({ name, props });
    },
  };
});

import { useWizardStore } from "../hooks/useWizardNavigation";
import { predictExport } from "../lib/speechRate";

const steps = () => tracked.filter((t) => t.name === "step_visited");

describe("step_visited is not emitted for navigations that do not happen", () => {
  beforeEach(() => {
    tracked.length = 0;
    useWizardStore.getState().reset();
  });

  it("goBack at the first step emits nothing", () => {
    // Observed: `step_visited` was 43% of all events, inflated by dead-end
    // navigation. goToStep always guarded this; goNext/goBack did not.
    useWizardStore.getState().goBack();
    expect(steps()).toHaveLength(0);
    expect(useWizardStore.getState().currentStep).toBe(0);
  });

  it("goNext at the last step emits nothing", () => {
    useWizardStore.getState().goToStep(5);
    tracked.length = 0;
    useWizardStore.getState().goNext();
    expect(steps()).toHaveLength(0);
    expect(useWizardStore.getState().currentStep).toBe(5);
  });

  it("goToStep onto the current step emits nothing", () => {
    useWizardStore.getState().goToStep(0);
    expect(steps()).toHaveLength(0);
  });

  it("still reports real navigation", () => {
    useWizardStore.getState().goNext();
    useWizardStore.getState().goBack();
    expect(steps().map((s) => s.props?.step)).toEqual(["Edit Video", "Project Setup"]);
  });
});

describe("predictExport reports how much of the video the script covers", () => {
  const seg = (start: number, end: number, text: string) => ({
    start_seconds: start,
    end_seconds: end,
    text,
  });

  it("reports the furthest segment end, not the last one in array order", () => {
    // A hand-edited or model-reordered script must not under-report its reach.
    const out = predictExport(
      [seg(0, 10, "one two three"), seg(40, 50, "four five six"), seg(20, 30, "seven eight")],
      "en",
      60,
    );
    expect(out.scriptCoverageSeconds).toBe(50);
  });

  it("surfaces the deflated-duration case that used to pass silently", () => {
    // The production shape: a 220s video whose script stopped at 53s.
    const segments = [
      seg(0, 12, "aa bb cc dd"),
      seg(12, 28, "ee ff gg hh"),
      seg(28, 40, "ii jj kk ll"),
      seg(40, 53, "mm nn oo pp"),
    ];
    const out = predictExport(segments, "en", 220);
    expect(out.scriptCoverageSeconds).toBe(53);
    // Nothing is scheduled past the end, which is exactly why the old
    // segmentsPastEnd signal could not catch this.
    expect(out.segmentsPastEnd).toBe(0);
    expect(out.scriptCoverageSeconds).toBeLessThan(220 * 0.6);
  });

  it("does not flag a script that covers the video", () => {
    const out = predictExport([seg(0, 55, "aa bb cc"), seg(55, 110, "dd ee ff")], "en", 115);
    expect(out.scriptCoverageSeconds).toBe(110);
    expect(out.scriptCoverageSeconds).toBeGreaterThan(115 * 0.6);
  });

  it("keeps coverage at zero for an empty script rather than reporting a false gap", () => {
    const out = predictExport([], "en", 120);
    expect(out.scriptCoverageSeconds).toBe(0);
  });
});
