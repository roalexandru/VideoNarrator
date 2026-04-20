//! End-to-end tests for the `narrator-cli` binary.
//!
//! These spawn the actual CLI subprocess against a tiny fixture video that
//! ffmpeg generates on the fly. They serve as the harness for re-architecture
//! work (Phases 3–6): when the compositor swaps in, these tests catch any
//! regression in the CLI surface without depending on the GUI.
//!
//! Tests are skipped (not failed) if ffmpeg is unavailable so CI environments
//! without ffmpeg don't false-fail.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Resolve the freshly built `narrator-cli` binary path. Cargo sets
/// `CARGO_BIN_EXE_<name>` for any `[[bin]]` target the test crate
/// declares an integration test for.
fn cli_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_narrator-cli"))
}

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Generate a tiny 2-second 320x240 30fps test video with a colour pattern.
fn make_fixture_video(dir: &Path) -> PathBuf {
    let path = dir.join("fixture.mp4");
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=2:size=320x240:rate=30",
            "-pix_fmt",
            "yuv420p",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
        ])
        .arg(&path)
        .status()
        .expect("spawn ffmpeg");
    assert!(status.success(), "ffmpeg fixture gen failed");
    path
}

/// Parse the single-line JSON envelope on stdout into a serde_json Value.
fn parse_envelope(stdout: &[u8]) -> serde_json::Value {
    let s = std::str::from_utf8(stdout).expect("stdout utf8");
    let line = s.lines().last().unwrap_or("").trim();
    serde_json::from_str(line).unwrap_or_else(|e| panic!("envelope parse failed: {e}; raw={s:?}"))
}

#[test]
fn cli_probe_video_returns_metadata() {
    if !ffmpeg_available() {
        eprintln!("skip: ffmpeg not on PATH");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let video = make_fixture_video(tmp.path());

    let output = Command::new(cli_path())
        .args(["probe", "video", "--input"])
        .arg(&video)
        .output()
        .expect("spawn cli");

    assert!(
        output.status.success(),
        "exit={:?} stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let env = parse_envelope(&output.stdout);
    assert_eq!(env["ok"], true);
    let data = &env["data"];
    assert_eq!(data["width"], 320);
    assert_eq!(data["height"], 240);
    assert!((data["duration_seconds"].as_f64().unwrap() - 2.0).abs() < 0.5);
}

#[test]
fn cli_apply_edits_renders_output_file() {
    if !ffmpeg_available() {
        eprintln!("skip: ffmpeg not on PATH");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let video = make_fixture_video(tmp.path());
    let plan = tmp.path().join("plan.json");
    let out = tmp.path().join("out.mp4");

    // Trivial plan: trim to first 1s, no effects.
    std::fs::write(
        &plan,
        r#"{
            "clips": [{
                "start_seconds": 0.0,
                "end_seconds": 1.0,
                "speed": 1.0,
                "fps_override": null
            }]
        }"#,
    )
    .unwrap();

    let output = Command::new(cli_path())
        .args(["render", "apply-edits", "--input"])
        .arg(&video)
        .arg("--plan")
        .arg(&plan)
        .arg("--output")
        .arg(&out)
        .output()
        .expect("spawn cli");

    assert!(
        output.status.success(),
        "exit={:?} stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let env = parse_envelope(&output.stdout);
    assert_eq!(env["ok"], true, "envelope: {env}");
    assert!(out.exists(), "output mp4 should exist");
    let sz = std::fs::metadata(&out).unwrap().len();
    assert!(sz > 1000, "output too small: {sz} bytes");
}

#[test]
fn cli_progress_json_emits_ndjson_to_stderr() {
    if !ffmpeg_available() {
        eprintln!("skip: ffmpeg not on PATH");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let video = make_fixture_video(tmp.path());
    let plan = tmp.path().join("plan.json");
    let out = tmp.path().join("out.mp4");

    std::fs::write(
        &plan,
        r#"{
            "clips": [{
                "start_seconds": 0.0,
                "end_seconds": 1.5,
                "speed": 2.0,
                "fps_override": null
            }]
        }"#,
    )
    .unwrap();

    let output = Command::new(cli_path())
        .args(["--progress", "json", "render", "apply-edits", "--input"])
        .arg(&video)
        .arg("--plan")
        .arg(&plan)
        .arg("--output")
        .arg(&out)
        .output()
        .expect("spawn cli");

    assert!(output.status.success(), "exit={:?}", output.status.code());

    let stderr = String::from_utf8_lossy(&output.stderr);
    let progress_lines: Vec<_> = stderr.lines().filter(|l| l.contains("\"kind\"")).collect();
    assert!(
        !progress_lines.is_empty(),
        "expected at least one progress NDJSON line on stderr, got: {stderr:?}"
    );
    for line in progress_lines {
        let v: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("ndjson parse: {e}; line={line}"));
        assert!(v["kind"].is_string());
    }
}

#[test]
fn cli_extract_single_frame_writes_image() {
    if !ffmpeg_available() {
        eprintln!("skip: ffmpeg not on PATH");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let video = make_fixture_video(tmp.path());
    let out = tmp.path().join("frame.jpg");

    let output = Command::new(cli_path())
        .args(["render", "extract-frame", "--input"])
        .arg(&video)
        .args(["--at", "1.0", "--output"])
        .arg(&out)
        .output()
        .expect("spawn cli");

    assert!(output.status.success(), "exit={:?}", output.status.code());
    assert!(out.exists());
    assert!(std::fs::metadata(&out).unwrap().len() > 200);
}

#[test]
fn cli_invalid_input_returns_error_envelope() {
    let output = Command::new(cli_path())
        .args([
            "probe",
            "video",
            "--input",
            "/nonexistent/path/to/missing.mp4",
        ])
        .output()
        .expect("spawn cli");

    assert!(!output.status.success(), "should fail on missing file");
    let env = parse_envelope(&output.stdout);
    assert_eq!(env["ok"], false);
    assert!(env["error"]["message"].is_string());
}

#[test]
fn cli_plan_via_stdin() {
    if !ffmpeg_available() {
        eprintln!("skip: ffmpeg not on PATH");
        return;
    }
    use std::io::Write;
    use std::process::Stdio;

    let tmp = tempfile::tempdir().unwrap();
    let video = make_fixture_video(tmp.path());
    let out = tmp.path().join("out.mp4");

    let mut child = Command::new(cli_path())
        .args(["render", "apply-edits", "--input"])
        .arg(&video)
        .args(["--plan", "-", "--output"])
        .arg(&out)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cli");

    let plan = r#"{"clips":[{"start_seconds":0,"end_seconds":1,"speed":1.0,"fps_override":null}]}"#;
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(plan.as_bytes())
        .unwrap();
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("cli wait");

    assert!(output.status.success(), "exit={:?}", output.status.code());
    assert!(out.exists());
}
