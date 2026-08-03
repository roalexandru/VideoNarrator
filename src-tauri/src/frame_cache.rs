//! Reuse of extracted frames across regenerations.
//!
//! Regenerating narration for an unchanged video re-ran the whole analysis
//! front end: two full-decode ffmpeg passes (scene detect + silence detect),
//! then up to 300 individual frame extractions. That is where the iterate-on-
//! prompt loop spent most of its wall clock, and none of it depended on the
//! prompt.
//!
//! Frames already persist per project at `~/.narrator/projects/<id>/frames/`,
//! so this does not introduce a second copy on disk. It adds a marker file
//! beside them recording *what those frames are*: which source they came from,
//! which sampling settings produced them, and the silence map discovered on the
//! way. A later run whose source and settings match reads the marker and skips
//! extraction entirely.
//!
//! ## Invalidation
//!
//! The key is the source's content hash (`compute_media_hash` — size plus 1 MiB
//! from each end) combined with the sampling settings. Anything that would
//! change which pixels the model sees changes the key:
//!
//!   - a different or edited video → different content hash
//!   - a different frame density, cap, or scene threshold → different settings
//!
//! Editing a video in place is the case a naive mtime check would get wrong and
//! a content hash gets right.
//!
//! Every frame file is also confirmed to still exist before a hit is reported,
//! so a partially-deleted frames directory falls back to a clean re-extract
//! rather than handing the model a short timeline.

use crate::error::NarratorError;
use crate::models::{Frame, FrameConfig, SilenceSpan};
use crate::video_engine::FrameExtraction;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Marker file name, written alongside the frames it describes.
const MARKER: &str = "extraction.json";

/// Bumped when the marker's meaning changes in a way older files can't satisfy.
const MARKER_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionMarker {
    pub version: u32,
    /// Content hash of the source video (`commands::compute_media_hash`).
    pub media_hash: String,
    /// Serialized sampling settings — see [`settings_key`].
    pub settings: String,
    /// Frame file names, relative to the marker's own directory.
    ///
    /// Stored as names rather than absolute paths so a moved or renamed
    /// `~/.narrator` still resolves, and so the marker cannot smuggle a path
    /// outside its directory.
    pub frame_files: Vec<String>,
    pub timestamps: Vec<f64>,
    pub widths: Vec<u32>,
    pub heights: Vec<u32>,
    pub silence_spans: Vec<SilenceSpan>,
    pub created_at: String,
}

/// Stable string describing every setting that affects which frames are picked.
///
/// `skip_dedup` is deliberately excluded: it only controls a post-extraction
/// filter that the app always disables, so including it would cause spurious
/// misses between GUI and CLI callers.
pub fn settings_key(config: &FrameConfig) -> String {
    format!(
        "v{MARKER_VERSION}:density={:?}:max={}:scene={:.3}",
        config.density, config.max_frames, config.scene_threshold
    )
}

/// Read a usable extraction for `(media_hash, config)` out of `frames_dir`.
///
/// Returns `None` — never an error — on any mismatch, unreadable marker, or
/// missing frame file. A cache miss must always be recoverable by re-extracting.
pub fn load(frames_dir: &Path, media_hash: &str, config: &FrameConfig) -> Option<FrameExtraction> {
    let marker_path = frames_dir.join(MARKER);
    let raw = std::fs::read_to_string(&marker_path).ok()?;
    let marker: ExtractionMarker = serde_json::from_str(&raw).ok()?;

    if marker.version != MARKER_VERSION {
        tracing::debug!("frame cache: marker version {} rejected", marker.version);
        return None;
    }
    if marker.media_hash != media_hash {
        tracing::info!("frame cache: source changed, re-extracting");
        return None;
    }
    if marker.settings != settings_key(config) {
        tracing::info!(
            "frame cache: sampling settings changed ({} → {}), re-extracting",
            marker.settings,
            settings_key(config)
        );
        return None;
    }

    // Every parallel array must agree, or the marker is corrupt.
    let n = marker.frame_files.len();
    if n == 0
        || marker.timestamps.len() != n
        || marker.widths.len() != n
        || marker.heights.len() != n
    {
        tracing::warn!("frame cache: marker arrays inconsistent, re-extracting");
        return None;
    }

    let mut frames = Vec::with_capacity(n);
    for i in 0..n {
        let name = &marker.frame_files[i];
        // Reject anything that isn't a plain file name: a marker is data on
        // disk, and `../` in it must not reach outside the frames directory.
        if name.is_empty() || Path::new(name).file_name().map(|f| f != name.as_str()) != Some(false)
        {
            tracing::warn!("frame cache: rejecting suspicious frame name {name:?}");
            return None;
        }
        let path = frames_dir.join(name);
        if !path.is_file() {
            tracing::info!("frame cache: {name} is gone, re-extracting");
            return None;
        }
        frames.push(Frame {
            index: i,
            timestamp_seconds: marker.timestamps[i],
            path,
            width: marker.widths[i],
            height: marker.heights[i],
        });
    }

    tracing::info!(
        "frame cache: reusing {} frames and {} silence spans",
        frames.len(),
        marker.silence_spans.len()
    );
    Some(FrameExtraction {
        frames,
        silence_spans: marker.silence_spans,
    })
}

/// Write the marker describing `extraction`.
///
/// Best-effort: a failure here costs a re-extract next time, which is strictly
/// better than failing the generation the user is waiting on.
pub fn store(
    frames_dir: &Path,
    media_hash: &str,
    config: &FrameConfig,
    extraction: &FrameExtraction,
) {
    if extraction.frames.is_empty() {
        return;
    }
    // A frame stored outside `frames_dir` could not be resolved on load, so
    // don't write a marker that would always miss.
    let mut frame_files = Vec::with_capacity(extraction.frames.len());
    for frame in &extraction.frames {
        match frame.path.file_name().and_then(|f| f.to_str()) {
            Some(name) if frame.path.parent() == Some(frames_dir) => {
                frame_files.push(name.to_string())
            }
            _ => {
                tracing::debug!(
                    "frame cache: not caching, {} is not directly in {}",
                    frame.path.display(),
                    frames_dir.display()
                );
                return;
            }
        }
    }

    let marker = ExtractionMarker {
        version: MARKER_VERSION,
        media_hash: media_hash.to_string(),
        settings: settings_key(config),
        frame_files,
        timestamps: extraction
            .frames
            .iter()
            .map(|f| f.timestamp_seconds)
            .collect(),
        widths: extraction.frames.iter().map(|f| f.width).collect(),
        heights: extraction.frames.iter().map(|f| f.height).collect(),
        silence_spans: extraction.silence_spans.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    match serde_json::to_string_pretty(&marker) {
        Ok(json) => {
            if let Err(e) = std::fs::write(frames_dir.join(MARKER), json) {
                tracing::warn!("frame cache: could not write marker: {e}");
            }
        }
        Err(e) => tracing::warn!("frame cache: could not serialize marker: {e}"),
    }
}

/// Base64-encode frames for the model, in parallel.
///
/// Encoding is a Lanczos3 downscale plus JPEG re-compression per frame — pure
/// CPU, and it ran serially for up to 300 frames. Rayon spreads it across cores;
/// callers are already inside `spawn_blocking`, so blocking here is correct.
///
/// Returns encodings in input order — rayon's completion order is
/// nondeterministic but `collect` into a `Result<Vec<_>>` preserves position.
/// A frame that fails to encode aborts the batch rather than being dropped: a
/// silently short frame list would make the model narrate a timeline it never
/// saw. Callers that want missing files skipped should filter first.
pub fn encode_frames_parallel(paths: &[PathBuf]) -> Result<Vec<String>, NarratorError> {
    use rayon::prelude::*;

    paths
        .par_iter()
        .map(|path| crate::video_engine::frame_to_base64(path))
        .collect()
}

/// Absolute path of the marker inside `frames_dir`.
#[cfg(test)]
fn marker_path(frames_dir: &Path) -> PathBuf {
    frames_dir.join(MARKER)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::FrameDensity;

    fn cfg() -> FrameConfig {
        FrameConfig {
            density: FrameDensity::Medium,
            scene_threshold: 0.3,
            max_frames: 30,
            skip_dedup: true,
        }
    }

    /// Write `count` fake JPEGs into `dir` and return the matching extraction.
    fn seed_frames(dir: &Path, count: usize) -> FrameExtraction {
        let mut frames = Vec::new();
        for i in 0..count {
            let name = format!("frame_{:04}.jpg", i + 1);
            let path = dir.join(&name);
            std::fs::write(&path, b"\xFF\xD8\xFF\xE0not-a-real-jpeg").unwrap();
            frames.push(Frame {
                index: i,
                timestamp_seconds: i as f64 * 2.5,
                path,
                width: 1920,
                height: 1080,
            });
        }
        FrameExtraction {
            frames,
            silence_spans: vec![
                SilenceSpan {
                    start: 1.0,
                    end: 2.0,
                },
                SilenceSpan {
                    start: 8.0,
                    end: 9.5,
                },
            ],
        }
    }

    #[test]
    fn round_trips_frames_and_silence_spans() {
        let dir = tempfile::tempdir().unwrap();
        let extraction = seed_frames(dir.path(), 3);
        store(dir.path(), "hash-a", &cfg(), &extraction);

        let loaded = load(dir.path(), "hash-a", &cfg()).expect("cache must hit");
        assert_eq!(loaded.frames.len(), 3);
        assert_eq!(loaded.frames[1].timestamp_seconds, 2.5);
        assert_eq!(loaded.frames[2].width, 1920);
        // The silence map is the part that would otherwise cost a full decode.
        assert_eq!(loaded.silence_spans.len(), 2);
        assert_eq!(loaded.silence_spans[1].end, 9.5);
        // Indices are renumbered densely so downstream frame_refs stay valid.
        for (i, f) in loaded.frames.iter().enumerate() {
            assert_eq!(f.index, i);
        }
    }

    #[test]
    fn misses_when_the_source_changed() {
        let dir = tempfile::tempdir().unwrap();
        store(dir.path(), "hash-a", &cfg(), &seed_frames(dir.path(), 2));
        assert!(
            load(dir.path(), "hash-b", &cfg()).is_none(),
            "a different source must not reuse frames"
        );
    }

    #[test]
    fn misses_when_sampling_settings_changed() {
        let dir = tempfile::tempdir().unwrap();
        store(dir.path(), "hash-a", &cfg(), &seed_frames(dir.path(), 2));

        // Density change → different frames entirely.
        let mut denser = cfg();
        denser.density = FrameDensity::Heavy;
        assert!(load(dir.path(), "hash-a", &denser).is_none());

        // Frame cap change.
        let mut capped = cfg();
        capped.max_frames = 120;
        assert!(load(dir.path(), "hash-a", &capped).is_none());

        // Scene threshold change.
        let mut sensitive = cfg();
        sensitive.scene_threshold = 0.15;
        assert!(load(dir.path(), "hash-a", &sensitive).is_none());
    }

    #[test]
    fn skip_dedup_does_not_affect_the_key() {
        // It only gates a post-extraction filter, so flipping it must not
        // invalidate — otherwise GUI and CLI callers would never share a cache.
        let dir = tempfile::tempdir().unwrap();
        store(dir.path(), "hash-a", &cfg(), &seed_frames(dir.path(), 2));
        let mut other = cfg();
        other.skip_dedup = !other.skip_dedup;
        assert!(load(dir.path(), "hash-a", &other).is_some());
    }

    #[test]
    fn misses_when_a_frame_file_was_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let extraction = seed_frames(dir.path(), 3);
        store(dir.path(), "hash-a", &cfg(), &extraction);

        std::fs::remove_file(&extraction.frames[1].path).unwrap();
        assert!(
            load(dir.path(), "hash-a", &cfg()).is_none(),
            "a partial frames dir must re-extract, not hand over a short timeline"
        );
    }

    #[test]
    fn misses_on_absent_corrupt_or_stale_marker() {
        let dir = tempfile::tempdir().unwrap();
        // No marker at all.
        assert!(load(dir.path(), "hash-a", &cfg()).is_none());

        // Not JSON.
        std::fs::write(marker_path(dir.path()), b"not json").unwrap();
        assert!(load(dir.path(), "hash-a", &cfg()).is_none());

        // Valid JSON, wrong version.
        let stale = serde_json::json!({
            "version": MARKER_VERSION + 1,
            "media_hash": "hash-a",
            "settings": settings_key(&cfg()),
            "frame_files": ["frame_0001.jpg"],
            "timestamps": [0.0], "widths": [10], "heights": [10],
            "silence_spans": [], "created_at": "2026-01-01T00:00:00Z"
        });
        std::fs::write(marker_path(dir.path()), stale.to_string()).unwrap();
        assert!(load(dir.path(), "hash-a", &cfg()).is_none());
    }

    #[test]
    fn misses_when_marker_arrays_disagree() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("frame_0001.jpg"), b"x").unwrap();
        let corrupt = serde_json::json!({
            "version": MARKER_VERSION,
            "media_hash": "hash-a",
            "settings": settings_key(&cfg()),
            "frame_files": ["frame_0001.jpg", "frame_0002.jpg"],
            // Only one timestamp for two files.
            "timestamps": [0.0], "widths": [10, 10], "heights": [10, 10],
            "silence_spans": [], "created_at": "2026-01-01T00:00:00Z"
        });
        std::fs::write(marker_path(dir.path()), corrupt.to_string()).unwrap();
        assert!(load(dir.path(), "hash-a", &cfg()).is_none());
    }

    #[test]
    fn rejects_a_marker_that_escapes_its_directory() {
        // A marker is data on disk. A traversal entry must not resolve.
        let dir = tempfile::tempdir().unwrap();
        for name in ["../outside.jpg", "sub/frame.jpg", ""] {
            let evil = serde_json::json!({
                "version": MARKER_VERSION,
                "media_hash": "hash-a",
                "settings": settings_key(&cfg()),
                "frame_files": [name],
                "timestamps": [0.0], "widths": [10], "heights": [10],
                "silence_spans": [], "created_at": "2026-01-01T00:00:00Z"
            });
            std::fs::write(marker_path(dir.path()), evil.to_string()).unwrap();
            assert!(
                load(dir.path(), "hash-a", &cfg()).is_none(),
                "must reject frame name {name:?}"
            );
        }
    }

    #[test]
    fn storing_an_empty_extraction_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        store(
            dir.path(),
            "hash-a",
            &cfg(),
            &FrameExtraction {
                frames: Vec::new(),
                silence_spans: Vec::new(),
            },
        );
        assert!(!marker_path(dir.path()).exists());
    }

    #[test]
    fn does_not_cache_frames_stored_elsewhere() {
        // Frames extracted to a temp work dir but recorded against a different
        // frames dir would never resolve on load, so no marker should be written.
        let frames_dir = tempfile::tempdir().unwrap();
        let other_dir = tempfile::tempdir().unwrap();
        let extraction = seed_frames(other_dir.path(), 2);
        store(frames_dir.path(), "hash-a", &cfg(), &extraction);
        assert!(!marker_path(frames_dir.path()).exists());
    }

    #[test]
    fn parallel_encode_preserves_frame_order() {
        // Rayon completion order is nondeterministic; the output must not be.
        // Distinguishable 1x1 JPEGs: encode each to a different pixel value.
        let dir = tempfile::tempdir().unwrap();
        let mut frames = Vec::new();
        for i in 0..8u8 {
            let path = dir.path().join(format!("f{i}.jpg"));
            let img = image::RgbImage::from_pixel(4, 4, image::Rgb([i * 30, 0, 0]));
            img.save(&path).unwrap();
            frames.push(Frame {
                index: i as usize,
                timestamp_seconds: i as f64,
                path,
                width: 4,
                height: 4,
            });
        }

        let paths: Vec<PathBuf> = frames.iter().map(|f| f.path.clone()).collect();
        let encoded = encode_frames_parallel(&paths).expect("all frames encode");
        assert_eq!(encoded.len(), 8);
        // Compare against a serial encode of the same files.
        let serial: Vec<String> = frames
            .iter()
            .map(|f| crate::video_engine::frame_to_base64(&f.path).unwrap())
            .collect();
        assert_eq!(encoded, serial, "parallel encode must match serial order");
    }

    #[test]
    fn parallel_encode_fails_loudly_on_an_unreadable_frame() {
        // Silently dropping a frame would make the model narrate a timeline it
        // never saw.
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good.jpg");
        image::RgbImage::from_pixel(4, 4, image::Rgb([1, 2, 3]))
            .save(&good)
            .unwrap();
        let paths = vec![good, dir.path().join("missing.jpg")];
        assert!(encode_frames_parallel(&paths).is_err());
    }
}
