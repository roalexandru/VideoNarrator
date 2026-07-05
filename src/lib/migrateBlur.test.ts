import { describe, it, expect } from "vitest";
import { migrateLegacyBlurRadius } from "./migrateBlur";

describe("migrateLegacyBlurRadius", () => {
  it("leaves already-normalized values (<= 1) unchanged", () => {
    expect(migrateLegacyBlurRadius(0.03, 1920)).toBe(0.03);
    expect(migrateLegacyBlurRadius(1, 1920)).toBe(1);
  });

  it("converts a legacy pixel radius so export sigma is preserved (n = 2r/W)", () => {
    // Old export sigma = r px; new sigma = n*W/2. n = 2r/W ⇒ new sigma = r.
    expect(migrateLegacyBlurRadius(20, 1920)).toBeCloseTo((2 * 20) / 1920, 6);
  });

  it("clamps pathological legacy radii to 0.5", () => {
    expect(migrateLegacyBlurRadius(5000, 1920)).toBe(0.5);
  });

  it("falls back to 1920 width when output width is unknown", () => {
    expect(migrateLegacyBlurRadius(20, 0)).toBeCloseTo((2 * 20) / 1920, 6);
  });
});
