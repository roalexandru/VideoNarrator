/**
 * Blur `radius` changed from absolute output-resolution pixels (interpreted as
 * a Gaussian sigma by the export) to a normalized fraction of video width (a
 * CSS blur radius). Legacy projects stored values > 1; convert them on load.
 *
 * The old export sigma for a legacy radius `r` was `r` pixels. The new export
 * derives sigma from the normalized value `n` as `n * outputW / 2`. Setting
 * `n = 2 * r / outputW` makes the new sigma equal the old one exactly, so a
 * migrated project's EXPORT is bit-comparable to before. (The old *preview*
 * strength depended on the user's window size and is unrecoverable — the now
 * resolution-independent preview simply shows the true strength.)
 *
 * Values already in (0, 1] are left untouched. The result is clamped to 0.5 so
 * a pathological legacy radius can't produce an absurd normalized value.
 */
export function migrateLegacyBlurRadius(radius: number, outputWidth: number): number {
  if (radius <= 1) return radius;
  const w = outputWidth > 0 ? outputWidth : 1920;
  return Math.min(0.5, (2 * radius) / w);
}
