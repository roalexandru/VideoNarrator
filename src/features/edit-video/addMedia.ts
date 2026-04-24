/**
 * Helpers for the "+" button on the timeline: import a video or image file,
 * register it as a MediaRef in the edit store's media pool, and append a
 * matching EditClip.
 *
 * Keeps all the probing + pool plumbing in one place so the TSX can stay
 * focused on the dialog + UX.
 */

import { convertFileSrc } from "@tauri-apps/api/core";
import { computeMediaHash, probeVideo } from "../../lib/tauri/commands";
import { useEditStore, type MediaRef } from "../../stores/editStore";

const IMAGE_EXTS = new Set(["jpg", "jpeg", "png", "gif", "webp", "bmp"]);
const VIDEO_EXTS = new Set(["mp4", "mov", "avi", "mkv", "webm", "m4v"]);

function extOf(path: string): string {
  const m = /\.([a-z0-9]+)$/i.exec(path);
  return m ? m[1].toLowerCase() : "";
}

function detectKind(path: string): "video" | "image" | null {
  const ext = extOf(path);
  if (IMAGE_EXTS.has(ext)) return "image";
  if (VIDEO_EXTS.has(ext)) return "video";
  return null;
}

/** Resolve natural dimensions of an image file by letting the browser load
 *  it. Fast for local files (Tauri serves them via convertFileSrc). */
async function probeImageDimensions(path: string): Promise<{ width: number; height: number }> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve({ width: img.naturalWidth, height: img.naturalHeight });
    img.onerror = () => reject(new Error(`Failed to decode image: ${path}`));
    img.src = convertFileSrc(path);
  });
}

/** Import a file, register its MediaRef, and append an EditClip to the
 *  timeline. Returns true on success. Caller is expected to catch any
 *  error and surface it via the existing error toast path. */
export async function importAndAddMedia(path: string): Promise<boolean> {
  const kind = detectKind(path);
  if (!kind) throw new Error(`Unsupported file type: ${path}`);

  // Compute a content-aware fingerprint so the same file imported from
  // two different paths (or re-imported after being moved) dedupes to one
  // MediaRef. blake3(size || head || tail) — see commands::compute_media_hash.
  const hash = await computeMediaHash(path);

  const store = useEditStore.getState();
  // Prefer hash match (strongest dedupe), fall back to path match for
  // legacy entries from before this field existed.
  const existing =
    Object.values(store.mediaPool).find((m) => m.hash === hash) ??
    Object.values(store.mediaPool).find((m) => m.path === path);

  let ref: MediaRef;
  if (existing) {
    ref = existing;
    // Heal stale path if the user moved the file — keep the same id so
    // existing clips don't lose their reference, but update path / dims.
    if (existing.path !== path) {
      ref = { ...existing, path };
      store.registerMedia(ref);
    }
  } else {
    if (kind === "video") {
      const meta = await probeVideo(path);
      ref = {
        id: crypto.randomUUID(),
        hash,
        kind: "video",
        path,
        duration: meta.duration_seconds,
        width: meta.width,
        height: meta.height,
        fps: meta.fps,
      };
    } else {
      const dims = await probeImageDimensions(path);
      ref = {
        id: crypto.randomUUID(),
        hash,
        kind: "image",
        path,
        duration: 0,
        width: dims.width,
        height: dims.height,
      };
    }
    store.registerMedia(ref);
  }

  if (ref.kind === "video") {
    store.addClip(ref.id, 0, ref.duration);
  } else {
    store.addImageClip(ref.id, 3.0);
  }
  return true;
}
