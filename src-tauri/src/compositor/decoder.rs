//! Decode a video file to raw RGBA frames via an `ffmpeg` subprocess.
//!
//! We pipe `ffmpeg -i input -f rawvideo -pix_fmt rgba -` and read fixed-size
//! W*H*4 byte chunks off stdout into a tokio mpsc channel. This sidesteps the
//! ffmpeg filtergraph entirely for the frame-by-frame compositor: ffmpeg is
//! used only to demux + decode + (if necessary) scale to the project's
//! output resolution. All effects happen in Rust on the decoded RGBA buffer.
//!
//! Frame ordering is preserved (sequential read). The decoder reports an
//! error if ffmpeg exits non-zero before EOF; callers that need partial
//! results should drain the receiver and inspect the join handle's result.

use std::path::Path;
use std::sync::Arc;

use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::error::NarratorError;
use crate::process_utils::CommandNoWindow;
use crate::video_engine;

/// One decoded frame: tightly packed RGBA8, length = `width * height * 4`.
/// The accompanying `width`/`height` are always the project's resolution
/// (the decoder requests it via the scale filter), so callers don't need to
/// re-probe.
pub struct RgbaFrame {
    pub data: Arc<Vec<u8>>,
    #[allow(dead_code)]
    pub width: u32,
    #[allow(dead_code)]
    pub height: u32,
}

/// Spawn an ffmpeg decoder for `path`, scaled to `(width, height)` at
/// `fps` frames/sec, decoding only the source range `[start_seconds,
/// end_seconds]`. Stream `RgbaFrame`s through the returned receiver;
/// the join handle resolves once stdout closes (check for ffmpeg exit
/// status if you need to confirm a clean decode).
///
/// `end_seconds = f64::INFINITY` means "decode to EOF".
pub async fn decode_video_range(
    path: &Path,
    start_seconds: f64,
    end_seconds: f64,
    width: u32,
    height: u32,
    fps: f64,
) -> Result<
    (
        mpsc::Receiver<RgbaFrame>,
        tokio::task::JoinHandle<Result<(), NarratorError>>,
    ),
    NarratorError,
> {
    let ffmpeg = video_engine::detect_ffmpeg()?;
    let frame_bytes = (width as usize) * (height as usize) * 4;
    let fps_arg = format!("{:.6}", fps);
    let scale_arg = format!("scale={width}:{height}:flags=lanczos,format=rgba");

    let mut cmd = Command::new(ffmpeg.as_os_str());
    cmd.no_window();
    cmd.args(["-hide_banner", "-loglevel", "error"]);
    if start_seconds > 0.0 {
        // -ss before -i: fast (keyframe) seek. Frame-accurate seek would be
        // slower; the typical user-perceptible drift is < 1 frame and the
        // existing pipeline accepted the same trade-off.
        cmd.args(["-ss", &format!("{:.6}", start_seconds)]);
    }
    cmd.arg("-i").arg(path.as_os_str());
    if end_seconds.is_finite() && end_seconds > start_seconds {
        let dur = (end_seconds - start_seconds).max(0.001);
        cmd.args(["-t", &format!("{:.6}", dur)]);
    }
    cmd.args([
        "-vf", &scale_arg, "-r", &fps_arg, "-f", "rawvideo", "-pix_fmt", "rgba", "-an", "-",
    ]);
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| NarratorError::FfmpegFailed(format!("decoder spawn: {e}")))?;

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| NarratorError::FfmpegFailed("decoder stdout".into()))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| NarratorError::FfmpegFailed("decoder stderr".into()))?;

    // bounded channel = backpressure if compositor is slower than decode
    let (tx, rx) = mpsc::channel::<RgbaFrame>(8);

    let handle = tokio::spawn(async move {
        let mut buf = vec![0u8; frame_bytes];
        loop {
            // Read exactly one frame's worth, or EOF.
            match stdout.read_exact(&mut buf).await {
                Ok(_) => {
                    let frame = RgbaFrame {
                        data: Arc::new(buf.clone()),
                        width,
                        height,
                    };
                    if tx.send(frame).await.is_err() {
                        // Receiver dropped — caller is no longer interested.
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => {
                    return Err(NarratorError::FfmpegFailed(format!("decoder read: {e}")));
                }
            }
        }
        // Drain stderr so we can include it in any error message.
        let mut err_msg = String::new();
        let _ = stderr.read_to_string(&mut err_msg).await;

        let status = child
            .wait()
            .await
            .map_err(|e| NarratorError::FfmpegFailed(format!("decoder wait: {e}")))?;
        if !status.success() {
            // Some success paths leave a non-zero status when stdout is
            // closed early by the consumer. Only fail if we genuinely never
            // produced a usable byte (caller already drained, so we just
            // surface the message).
            let tail = err_msg.lines().rev().take(5).collect::<Vec<_>>().join("\n");
            return Err(NarratorError::FfmpegFailed(format!(
                "decoder exited {:?}: {tail}",
                status.code()
            )));
        }
        Ok(())
    });

    Ok((rx, handle))
}

/// Decode a single frame at `time_seconds`, returning a tightly packed RGBA
/// buffer of length `width * height * 4`. Used for freeze clips.
pub async fn decode_single_frame_rgba(
    path: &Path,
    time_seconds: f64,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, NarratorError> {
    let ffmpeg = video_engine::detect_ffmpeg()?;
    let scale_arg = format!("scale={width}:{height}:flags=lanczos,format=rgba");
    let output = Command::new(ffmpeg.as_os_str())
        .no_window()
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-ss",
            &format!("{:.6}", time_seconds),
            "-i",
        ])
        .arg(path.as_os_str())
        .args([
            "-vf",
            &scale_arg,
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-an",
            "-",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| NarratorError::FfmpegFailed(format!("freeze decode: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(NarratorError::FfmpegFailed(format!(
            "freeze decode failed: {stderr}"
        )));
    }
    let expected = (width as usize) * (height as usize) * 4;
    if output.stdout.len() < expected {
        return Err(NarratorError::FfmpegFailed(format!(
            "freeze decode produced {} bytes, expected {expected}",
            output.stdout.len()
        )));
    }
    Ok(output.stdout[..expected].to_vec())
}
