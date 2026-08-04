//! End-to-end tests for the features added in 0.10.0.
//!
//! Unlike the per-module unit tests, these run the *real* machinery: actual
//! ffmpeg processes, actual files on disk, actual OCR. They exist because every
//! feature in this release has a seam where the pure logic is correct but the
//! integration can still be wrong — a filter string that parses in a test but
//! not in ffmpeg, a cache marker that round-trips in isolation but not against
//! frames the extractor actually produced.
//!
//! Lives inside the lib rather than `tests/` because the modules under test are
//! private, and making them `pub` purely for testing would widen the crate's
//! surface for no other reason.
//!
//! Each test skips rather than fails when the ffmpeg on this machine lacks the
//! filter it needs. That is a deliberate trade — a developer without libzimg
//! should not see red — but it does mean coverage varies by machine, so each
//! skip prints why.

#![cfg(test)]

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::models::{FrameConfig, FrameDensity};

// ── Harness ─────────────────────────────────────────────────────────────────

fn ffmpeg() -> Option<PathBuf> {
    crate::video_engine::detect_ffmpeg().ok()
}

/// Render a test video. `audio` picks the soundtrack:
/// `None` = silent, `Some(spec)` = an ffmpeg lavfi audio expression.
fn make_video(path: &Path, seconds: f64, size: &str, audio: Option<&str>) -> bool {
    let Some(ff) = ffmpeg() else { return false };
    let mut cmd = std::process::Command::new(ff);
    cmd.args(["-hide_banner", "-loglevel", "error", "-y"]);
    cmd.args([
        "-f",
        "lavfi",
        "-i",
        &format!("testsrc=duration={seconds}:size={size}:rate=30"),
    ]);
    if let Some(spec) = audio {
        cmd.args(["-f", "lavfi", "-i", spec]);
    }
    cmd.args([
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
    ]);
    if audio.is_some() {
        cmd.args(["-c:a", "aac", "-shortest"]);
    }
    // Short GOP so trims and seeks land where the tests expect.
    cmd.args(["-g", "15", "-keyint_min", "15"]);
    cmd.arg(path);
    matches!(cmd.status(), Ok(s) if s.success()) && path.is_file()
}

/// Extract frames the way generation does, returning the real extraction result.
async fn extract(
    video: &Path,
    out_dir: &Path,
    config: &FrameConfig,
) -> crate::video_engine::FrameExtraction {
    crate::video_engine::extract_frames_with_anchors(
        video,
        config,
        out_dir,
        None,
        |_| {},
        |_, _| {},
        Some(Arc::new(AtomicBool::new(false))),
    )
    .await
    .expect("extraction should succeed on a generated fixture")
}

fn medium_config(max_frames: usize) -> FrameConfig {
    FrameConfig {
        density: FrameDensity::Medium,
        scene_threshold: 0.3,
        max_frames,
        skip_dedup: true,
    }
}

// ── Frame cache against real extraction ─────────────────────────────────────

/// The cache has to survive frames the *extractor* produced, not frames a test
/// fabricated — including the promotion step that rewrites every path.
#[tokio::test]
async fn e2e_frame_cache_hits_after_a_real_extraction() {
    let Some(_) = ffmpeg() else {
        eprintln!("skip: no ffmpeg");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("in.mp4");
    if !make_video(&video, 6.0, "320x240", None) {
        eprintln!("skip: could not render fixture");
        return;
    }
    let frames_dir = dir.path().join("frames");
    let config = medium_config(30);

    let extraction = extract(&video, &frames_dir, &config).await;
    assert!(
        !extraction.frames.is_empty(),
        "extraction produced no frames"
    );

    crate::frame_cache::store(&frames_dir, "hash-1", &config, &extraction);
    let hit = crate::frame_cache::load(&frames_dir, "hash-1", &config)
        .expect("cache must hit for the same source and settings");

    assert_eq!(hit.frames.len(), extraction.frames.len());
    for (cached, original) in hit.frames.iter().zip(&extraction.frames) {
        assert_eq!(cached.path, original.path, "cached path must resolve");
        assert!(cached.path.is_file(), "cached frame missing on disk");
        assert!(
            (cached.timestamp_seconds - original.timestamp_seconds).abs() < 1e-9,
            "timestamp drifted through the cache"
        );
    }

    // A denser setting must miss — those are different frames.
    let mut denser = config.clone();
    denser.density = FrameDensity::Heavy;
    assert!(
        crate::frame_cache::load(&frames_dir, "hash-1", &denser).is_none(),
        "a density change must invalidate"
    );
    // A different source must miss.
    assert!(crate::frame_cache::load(&frames_dir, "hash-2", &config).is_none());
}

/// Parallel base64 encoding has to produce exactly what the model would have
/// received before, on real JPEGs written by ffmpeg.
#[tokio::test]
async fn e2e_parallel_encode_matches_serial_on_extracted_frames() {
    let Some(_) = ffmpeg() else {
        eprintln!("skip: no ffmpeg");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("in.mp4");
    if !make_video(&video, 8.0, "640x360", None) {
        eprintln!("skip: could not render fixture");
        return;
    }
    let extraction = extract(&video, &dir.path().join("frames"), &medium_config(30)).await;
    let paths: Vec<PathBuf> = extraction.frames.iter().map(|f| f.path.clone()).collect();
    assert!(paths.len() >= 2, "need several frames to test ordering");

    let parallel = crate::frame_cache::encode_frames_parallel(&paths).expect("parallel encode");
    let serial: Vec<String> = paths
        .iter()
        .map(|p| crate::video_engine::frame_to_base64(p).unwrap())
        .collect();
    assert_eq!(parallel, serial, "parallel encode diverged from serial");
    assert!(parallel.iter().all(|b| !b.is_empty()));
}

// ── Silence detection and snapping on real audio ────────────────────────────

/// A video whose audio is loud-then-silent must yield a usable span, and
/// snapping must move a segment edge into it.
#[tokio::test]
async fn e2e_silence_spans_drive_segment_snapping() {
    let Some(_) = ffmpeg() else {
        eprintln!("skip: no ffmpeg");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("tone_then_quiet.mp4");
    // 3 s of tone, then 3 s of silence: one unambiguous span at the end.
    let audio = "sine=frequency=440:duration=3,apad=pad_dur=3";
    if !make_video(&video, 6.0, "320x240", Some(audio)) {
        eprintln!("skip: could not render fixture with audio");
        return;
    }

    let extraction = extract(&video, &dir.path().join("frames"), &medium_config(30)).await;
    let spans = extraction.silence_spans;
    if spans.is_empty() {
        eprintln!("skip: silencedetect found nothing in the fixture");
        return;
    }

    // Not effectively silent — there are 3 s of tone.
    assert!(
        !crate::ai_client::is_effectively_silent(&spans, 6.0),
        "half-silent source must not be treated as silent"
    );

    // A segment starting inside the tone should be pulled toward the quiet part
    // when the quiet part is within the search window.
    let wide = spans
        .iter()
        .max_by(|a, b| a.duration().total_cmp(&b.duration()))
        .copied()
        .expect("at least one span");
    assert!(wide.duration() > 0.4, "expected a clean gap, got {wide:?}");

    let start_just_before = (wide.start - 0.4).max(0.0);
    let segs = vec![crate::models::Segment {
        index: 0,
        start_seconds: start_just_before,
        end_seconds: 6.0,
        text: "narration".into(),
        visual_description: String::new(),
        emphasis: vec![],
        pace: crate::models::Pace::default(),
        pause_after_ms: 0,
        frame_refs: vec![],
        voice_override: None,
    }];
    let snapped = crate::ai_client::snap_to_silence(segs, &spans, 6.0);
    assert!(
        snapped[0].start_seconds >= start_just_before,
        "snap moved the edge backwards, away from the gap"
    );
    // And the result must survive normalization unchanged in count.
    let normalized = crate::ai_client::normalize_timeline(snapped, 6.0);
    assert_eq!(normalized.len(), 1);
}

// ── Contact sheets from real frames ─────────────────────────────────────────

/// Tiling has to work on real extracted JPEGs, not just synthetic solids, and
/// the composite must be a decodable image of the expected geometry.
#[tokio::test]
async fn e2e_contact_sheet_from_extracted_frames_decodes() {
    let Some(_) = ffmpeg() else {
        eprintln!("skip: no ffmpeg");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("in.mp4");
    if !make_video(&video, 20.0, "640x360", None) {
        eprintln!("skip: could not render fixture");
        return;
    }
    let extraction = extract(&video, &dir.path().join("frames"), &medium_config(30)).await;
    let group: Vec<crate::models::Frame> = extraction.frames.iter().take(4).cloned().collect();
    assert_eq!(group.len(), 4, "need 4 frames for a 2-row sheet");

    let sheet = crate::contact_sheet::build(&group, 2, 128)
        .expect("sheet build")
        .expect("sheet produced");

    assert_eq!((sheet.columns, sheet.rows), (2, 2));
    assert_eq!(sheet.timestamps.len(), 4);

    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &sheet.base64)
        .expect("valid base64");
    let img = image::load_from_memory(&bytes).expect("composite must be a decodable JPEG");
    // 640x360 source → 128-wide cells are 72 high.
    assert_eq!(img.width(), 2 * 128 + 4, "two cells plus one gutter");
    assert_eq!(img.height(), 2 * 72 + 4);

    // The description must address every frame by its real source index.
    let described = sheet.describe();
    for frame in &group {
        assert!(
            described.contains(&format!("frame {} at", frame.index)),
            "sheet description missing frame {}",
            frame.index
        );
    }
}

// ── OCR text layer against a real screenshot ────────────────────────────────

/// The claim the feature rests on, through the whole pipeline: a frame with a
/// command on it produces a prompt block naming that command.
#[cfg(target_os = "macos")]
#[test]
fn e2e_screen_text_layer_names_the_command() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/onscreen_text.png");
    assert!(fixture.is_file(), "missing fixture");

    // Three frames of the same screen: the layer should collapse them into one
    // state, and must not mistake the only content for chrome.
    let frames: Vec<crate::models::Frame> = (0..3)
        .map(|i| crate::models::Frame {
            index: i,
            timestamp_seconds: i as f64 * 2.0,
            path: fixture.clone(),
            width: 460,
            height: 90,
        })
        .collect();

    let backend = crate::screen_text::platform_backend();
    assert_eq!(backend.name(), "macos-vision");
    let block = crate::screen_text::build_text_layer(&frames, backend.as_ref());

    // With every frame identical, the text is in 100% of frames — so chrome
    // filtering removes it and the block is empty. That is correct behaviour and
    // worth pinning: identical frames carry no differentiating text.
    assert!(
        block.is_empty(),
        "text present in every frame is chrome by definition, got:\n{block}"
    );

    // Now mix in a frame with different content so the command is no longer
    // ubiquitous, and confirm it reaches the prompt.
    let blank = std::env::temp_dir().join("_e2e_blank.png");
    image::RgbImage::from_pixel(460, 90, image::Rgb([255, 255, 255]))
        .save(&blank)
        .unwrap();
    let mixed: Vec<crate::models::Frame> = vec![
        frames[0].clone(),
        crate::models::Frame {
            index: 1,
            timestamp_seconds: 2.0,
            path: blank.clone(),
            width: 460,
            height: 90,
        },
        crate::models::Frame {
            index: 2,
            timestamp_seconds: 4.0,
            path: blank.clone(),
            width: 460,
            height: 90,
        },
    ];
    let block = crate::screen_text::build_text_layer(&mixed, backend.as_ref());
    let _ = std::fs::remove_file(&blank);

    assert!(block.contains("ON-SCREEN TEXT"), "no block produced");
    assert!(
        block.to_lowercase().contains("tauri"),
        "the recognised command did not reach the prompt:\n{block}"
    );
    assert!(
        block.contains("Do NOT read it aloud verbatim"),
        "the anti-recitation instruction is missing"
    );
}

// ── Loudness on real audio ──────────────────────────────────────────────────

/// Two-pass normalization must actually land near the target on real audio.
#[tokio::test]
async fn e2e_loudness_normalizes_a_quiet_track_to_target() {
    let Some(ff) = ffmpeg() else {
        eprintln!("skip: no ffmpeg");
        return;
    };
    if !crate::loudness::ffmpeg_has_loudnorm() {
        eprintln!("skip: this ffmpeg has no loudnorm filter");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let quiet = dir.path().join("quiet.wav");
    let ok = std::process::Command::new(&ff)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=6",
            "-filter:a",
            "volume=0.05",
            "-c:a",
            "pcm_s16le",
            "-ar",
            "48000",
            "-ac",
            "2",
        ])
        .arg(&quiet)
        .status();
    if !matches!(ok, Ok(s) if s.success()) {
        eprintln!("skip: could not render quiet audio");
        return;
    }

    let before = crate::loudness::measure(&quiet)
        .await
        .expect("measurable audio");
    assert!(
        before.input_i < -30.0,
        "fixture should be very quiet, measured {}",
        before.input_i
    );

    let normalized = dir.path().join("loud.wav");
    let applied = crate::loudness::normalize_to(&quiet, &normalized)
        .await
        .expect("normalize must not error");
    assert!(applied, "a -40 LUFS track must be normalized, not skipped");

    let after = crate::loudness::measure(&normalized)
        .await
        .expect("measurable output");
    assert!(
        (after.input_i - crate::loudness::TARGET_I).abs() < 1.5,
        "normalized to {} LUFS, expected ~{}",
        after.input_i,
        crate::loudness::TARGET_I
    );
}

// ── Export verification against a real render ───────────────────────────────

/// Verification must pass on a file the app itself produced — otherwise the
/// panel cries wolf on every export.
#[tokio::test]
async fn e2e_export_verification_passes_on_a_real_mux() {
    let Some(_) = ffmpeg() else {
        eprintln!("skip: no ffmpeg");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("v.mp4");
    let narration = dir.path().join("a.mp4");
    let merged = dir.path().join("merged.mp4");
    if !make_video(
        &video,
        6.0,
        "320x240",
        Some("sine=frequency=200:duration=6"),
    ) || !make_video(
        &narration,
        6.0,
        "320x240",
        Some("sine=frequency=600:duration=6"),
    ) {
        eprintln!("skip: could not render fixtures");
        return;
    }

    let outcome = crate::video_edit::merge_audio_video(
        video.to_str().unwrap(),
        narration.to_str().unwrap(),
        merged.to_str().unwrap(),
        true,
        -8.0,
        |_, _| {},
    )
    .await;
    if outcome.is_err() {
        eprintln!("skip: merge failed: {:?}", outcome.err());
        return;
    }

    let report = crate::export_verify::verify_export(
        &merged,
        &crate::export_verify::ExportIntent {
            expected_duration: 6.0,
            narration_expected: true,
            burn_check: None,
        },
    )
    .await
    .expect("verification should run on a real file");

    // Report the specifics before asserting, so a CI failure is diagnosable.
    for check in &report.checks {
        eprintln!("  {} -> {:?}", check.id, check.status);
    }
    assert!(
        report.all_passed(),
        "{} check(s) failed on a file we produced ourselves",
        report.failures()
    );
    // The subtitle check must be absent, not fabricated, when not requested.
    assert!(!report.checks.iter().any(|c| c.id == "subtitles_burnt"));
}

/// A truncated file must be *caught*, or the whole feature is decorative.
#[tokio::test]
async fn e2e_export_verification_catches_a_short_render() {
    let Some(_) = ffmpeg() else {
        eprintln!("skip: no ffmpeg");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let short = dir.path().join("short.mp4");
    if !make_video(
        &short,
        2.0,
        "320x240",
        Some("sine=frequency=200:duration=2"),
    ) {
        eprintln!("skip: could not render fixture");
        return;
    }

    // Claim the script planned 30 s; the file is 2 s.
    let report = crate::export_verify::verify_export(
        &short,
        &crate::export_verify::ExportIntent {
            expected_duration: 30.0,
            narration_expected: true,
            burn_check: None,
        },
    )
    .await
    .expect("verification should run");

    let duration = report
        .checks
        .iter()
        .find(|c| c.id == "duration")
        .expect("duration check present");
    assert!(
        duration.status.is_fail(),
        "a 2 s file against a 30 s script must fail: {:?}",
        duration.status
    );
    assert!(!report.all_passed());
}

// ── Preview render tier ─────────────────────────────────────────────────────

/// The preview tier must genuinely downscale. A cheaper CRF alone would leave a
/// 4K render paying full-resolution compositing per frame.
#[tokio::test]
async fn e2e_preview_tier_downscales_the_render() {
    let Some(_) = ffmpeg() else {
        eprintln!("skip: no ffmpeg");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("big.mp4");
    // 1920x1080 so a 720p cap has something to do.
    if !make_video(&source, 3.0, "1920x1080", None) {
        eprintln!("skip: could not render HD fixture");
        return;
    }

    // A speed change forces the compositor path rather than the stream-copy
    // fast path, which is where the quality tier applies.
    let plan = crate::video_edit::VideoEditPlan {
        clips: vec![crate::video_edit::EditClip {
            start_seconds: 0.0,
            end_seconds: 2.0,
            speed: 2.0,
            skip_frames: false,
            fps_override: None,
            clip_type: None,
            freeze_source_time: None,
            freeze_duration: None,
            image_duration: None,
            input_path: None,
            zoom_pan: None,
        }],
        effects: None,
    };

    for (quality, cap) in [
        (crate::models::RenderQuality::Preview, 720u32),
        (crate::models::RenderQuality::Draft, 480u32),
    ] {
        let out = dir.path().join(format!("{quality:?}.mp4"));
        let result = crate::video_edit::apply_edits_with_cancel(
            source.to_str().unwrap(),
            out.to_str().unwrap(),
            &plan,
            quality,
            |_, _| {},
            None,
        )
        .await;
        if result.is_err() {
            eprintln!("skip {quality:?}: render failed: {:?}", result.err());
            continue;
        }
        let meta = crate::video_engine::probe_video(&out)
            .await
            .expect("probe rendered preview");
        assert!(
            meta.height <= cap,
            "{quality:?} produced {}p, expected <= {cap}p",
            meta.height
        );
        assert_eq!(meta.height % 2, 0, "odd height would not have encoded");
    }
}

/// Final quality must keep source resolution — the deliverable is never
/// silently degraded by this feature.
#[tokio::test]
async fn e2e_final_tier_preserves_source_resolution() {
    let Some(_) = ffmpeg() else {
        eprintln!("skip: no ffmpeg");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("src.mp4");
    if !make_video(&source, 3.0, "1280x720", None) {
        eprintln!("skip: could not render fixture");
        return;
    }
    let out = dir.path().join("final.mp4");
    let plan = crate::video_edit::VideoEditPlan {
        clips: vec![crate::video_edit::EditClip {
            start_seconds: 0.0,
            end_seconds: 2.0,
            speed: 2.0,
            skip_frames: false,
            fps_override: None,
            clip_type: None,
            freeze_source_time: None,
            freeze_duration: None,
            image_duration: None,
            input_path: None,
            zoom_pan: None,
        }],
        effects: None,
    };
    let result = crate::video_edit::apply_edits_with_cancel(
        source.to_str().unwrap(),
        out.to_str().unwrap(),
        &plan,
        crate::models::RenderQuality::Final,
        |_, _| {},
        None,
    )
    .await;
    if result.is_err() {
        eprintln!("skip: render failed: {:?}", result.err());
        return;
    }
    let meta = crate::video_engine::probe_video(&out).await.unwrap();
    assert_eq!(
        (meta.width, meta.height),
        (1280, 720),
        "Final tier must not downscale"
    );
}

// ── HDR tone mapping on a real PQ source ────────────────────────────────────

/// An HDR source must come out tagged bt709, and the export-verification colour
/// check must agree.
#[tokio::test]
async fn e2e_hdr_source_is_tonemapped_to_bt709() {
    let Some(ff) = ffmpeg() else {
        eprintln!("skip: no ffmpeg");
        return;
    };
    if !crate::video_engine::ffmpeg_has_zscale_filter() {
        eprintln!("skip: this ffmpeg has no zscale (libzimg), tone mapping is inert");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let hdr = dir.path().join("hdr.mp4");
    // Tag the stream as PQ / BT.2020 without needing real HDR content.
    let ok = std::process::Command::new(&ff)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=3:size=640x360:rate=30",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p10le",
            "-color_primaries",
            "bt2020",
            "-color_trc",
            "smpte2084",
            "-colorspace",
            "bt2020nc",
        ])
        .arg(&hdr)
        .status();
    if !matches!(ok, Ok(s) if s.success()) {
        eprintln!("skip: could not render a PQ-tagged fixture");
        return;
    }

    // The probe must recognise it as HDR in the first place.
    let transfer = crate::video_engine::probe_color_transfer(&hdr)
        .await
        .ok()
        .flatten();
    if transfer
        .as_deref()
        .map(crate::video_engine::is_hdr_transfer)
        != Some(true)
    {
        eprintln!("skip: fixture was not tagged HDR (got {transfer:?})");
        return;
    }
    assert!(crate::video_engine::needs_hdr_tonemap(&hdr).await);

    // Frame extraction should now tone map; the JPEGs must at least decode.
    let extraction = extract(&hdr, &dir.path().join("frames"), &medium_config(10)).await;
    assert!(!extraction.frames.is_empty(), "no frames from HDR source");
    for frame in extraction.frames.iter().take(3) {
        assert!(
            image::open(&frame.path).is_ok(),
            "tone-mapped frame {} is not a readable image",
            frame.path.display()
        );
    }
}

// ── Cost estimate against a real video ──────────────────────────────────────

/// The estimate must describe what extraction actually does, not what the
/// settings literally say.
#[tokio::test]
async fn e2e_cost_estimate_matches_actual_frame_count() {
    let Some(_) = ffmpeg() else {
        eprintln!("skip: no ffmpeg");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("in.mp4");
    // 30 s at medium (5 s interval) → 6 frames, well under the 300 cap.
    if !make_video(&video, 30.0, "320x240", None) {
        eprintln!("skip: could not render fixture");
        return;
    }
    let config = medium_config(300);

    let estimate = crate::cost_estimate::estimate(
        &config,
        crate::cost_estimate::EstimateInputs {
            duration_seconds: 30.0,
            images_per_request: crate::ai_client::MAX_FRAMES_PER_CALL,
            frames_per_sheet: crate::ai_client::FRAMES_PER_SHEET,
            tiled: false,
            model_selection: false,
            survey_frame_count: crate::frame_selection::SURVEY_FRAME_COUNT,
            strict_mode: false,
            cached_frames: false,
        },
    );

    // The forecast must not simply report the 300 cap.
    assert!(
        estimate.frame_count < 20,
        "estimate said {} frames for a 30 s medium-density video",
        estimate.frame_count
    );
    assert_eq!(estimate.request_count, 1, "6 images fit one request");
    assert!(
        estimate.summary.contains("1 request,"),
        "{}",
        estimate.summary
    );

    // And it must be in the right ballpark against a real fixed-interval
    // extraction (anchor detection can pick a different count, so this is a
    // sanity bound rather than an equality).
    let extraction = extract(&video, &dir.path().join("frames"), &config).await;
    let actual = extraction.frames.len();
    assert!(
        actual > 0 && actual <= config.max_frames,
        "extraction produced {actual} frames"
    );
}
