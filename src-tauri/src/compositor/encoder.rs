//! Encode raw RGBA frames to an MP4 via an `ffmpeg` subprocess.
//!
//! ffmpeg is invoked as encoder-only: a single `-f rawvideo` input on stdin
//! plus an optional second input file for audio (`-c:a copy`). The compositor
//! writes one frame's worth of RGBA bytes per call to `write_frame`, ffmpeg
//! does the swscale → yuv420p conversion + libx264 encode + mp4 mux.
//!
//! Two-task layout: stdin write happens on the calling task; stderr is
//! drained on a background task so a stuck encoder can't deadlock by
//! filling its OS error pipe.

use std::path::{Path, PathBuf};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin};

use crate::error::NarratorError;
use crate::models::RenderQuality;
use crate::process_utils::CommandNoWindow;
use crate::video_engine;

pub struct Encoder {
    child: Child,
    stdin: Option<ChildStdin>,
    stderr_handle: Option<tokio::task::JoinHandle<String>>,
    /// Final destination, renamed into place only on a successful `finish()`.
    output_path: PathBuf,
    /// Sibling temp file ffmpeg actually writes to. Keeping the real output
    /// path untouched until the mux completes means a crashed/cancelled encode
    /// can never leave a truncated-but-valid MP4 at `output_path` (which would
    /// then pass the frontend's `fileExists` cache check).
    partial_path: PathBuf,
    finished: bool,
}

impl Drop for Encoder {
    fn drop(&mut self) {
        // `kill_on_drop(true)` reaps the ffmpeg child; here we just make sure a
        // half-written partial file doesn't linger. On the success path
        // `finish()` renames it away and clears `finished`.
        if !self.finished {
            let _ = std::fs::remove_file(&self.partial_path);
        }
    }
}

impl Encoder {
    /// Start an encoder writing to `output_path`. The audio source (if
    /// supplied) is re-encoded to AAC because the single-pass pipeline
    /// feeds it a PCM WAV (timeline-assembled) that the MP4 container
    /// can't hold natively.
    pub async fn start_with_aac(
        output_path: &Path,
        width: u32,
        height: u32,
        fps: f64,
        audio_source: Option<&Path>,
        quality: RenderQuality,
    ) -> Result<Self, NarratorError> {
        Self::start_inner(
            output_path,
            width,
            height,
            fps,
            audio_source,
            "aac",
            quality,
        )
        .await
    }

    async fn start_inner(
        output_path: &Path,
        width: u32,
        height: u32,
        fps: f64,
        audio_source: Option<&Path>,
        audio_codec: &str,
        quality: RenderQuality,
    ) -> Result<Self, NarratorError> {
        let ffmpeg = video_engine::detect_ffmpeg()?;
        let size_arg = format!("{width}x{height}");
        let fps_arg = format!("{:.6}", fps);

        // ffmpeg writes to a sibling `.partial-<uuid>.<ext>` file; we rename it
        // onto `output_path` only after a clean `finish()`. The extension is
        // preserved so container-format inference still works.
        let ext = output_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("mp4");
        let stem = output_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("out");
        let partial_path = output_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(format!(".{stem}.partial-{}.{ext}", uuid::Uuid::new_v4()));

        let mut cmd = tokio::process::Command::new(ffmpeg.as_os_str());
        cmd.no_window();
        // Reap the encoder ffmpeg if this Encoder is dropped without finishing.
        cmd.kill_on_drop(true);
        cmd.args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
            &size_arg,
            "-r",
            &fps_arg,
            "-i",
            "-",
        ]);

        if let Some(audio) = audio_source {
            cmd.arg("-i").arg(audio.as_os_str()).args([
                "-map",
                "0:v:0",
                "-map",
                "1:a:0?",
                "-c:v",
                "libx264",
                // Visually lossless, universally playable output. The previous
                // `ultrafast -crf 0` was true-lossless: 50-150 Mbps files that
                // QuickTime/AVFoundation and Windows Media Foundation refuse to
                // decode (qp=0 forces the High 4:4:4 Predictive profile). This
                // matters because merge_audio_video stream-copies (`-c:v copy`)
                // whatever we emit straight into the user's final export, so
                // this IS the delivered video. Same reasoning as burn_subtitles.
                "-preset",
                quality.preset(),
                "-crf",
                quality.crf(),
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                audio_codec,
            ]);
            if audio_codec == "aac" {
                cmd.args(["-b:a", "256k"]);
            }
            // No `-shortest`: freeze clips contribute video but not audio, so
            // the audio WAV is often shorter than the video stream. Letting
            // the longer (video) stream finish keeps the output's duration
            // matching the timeline; trailing frames simply have no audio.
            cmd.args(["-movflags", "+faststart"]);
        } else {
            cmd.args([
                "-map",
                "0:v:0",
                "-c:v",
                "libx264",
                // See the audio branch above: visually lossless CRF 18 instead
                // of true-lossless CRF 0, which produced huge, undecodable files.
                "-preset",
                quality.preset(),
                "-crf",
                quality.crf(),
                "-pix_fmt",
                "yuv420p",
                "-an",
                "-movflags",
                "+faststart",
            ]);
        }

        cmd.arg(partial_path.as_os_str())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| NarratorError::FfmpegFailed(format!("encoder spawn: {e}")))?;

        let stdin = child.stdin.take();
        let stderr_handle = child.stderr.take().map(|mut s| {
            tokio::spawn(async move {
                let mut buf = String::new();
                let _ = s.read_to_string(&mut buf).await;
                buf
            })
        });

        Ok(Self {
            child,
            stdin,
            stderr_handle,
            output_path: output_path.to_path_buf(),
            partial_path,
            finished: false,
        })
    }

    /// Write one RGBA frame (length = w*h*4) to the encoder.
    /// Errors if the encoder process has exited (broken pipe).
    pub async fn write_frame(&mut self, rgba: &[u8]) -> Result<(), NarratorError> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| NarratorError::FfmpegFailed("encoder stdin closed".into()))?;
        stdin.write_all(rgba).await.map_err(|e| {
            NarratorError::FfmpegFailed(format!("encoder write: {e} (encoder likely exited)"))
        })?;
        Ok(())
    }

    /// Flush + close stdin and wait for ffmpeg to finalize the file.
    /// Returns the resolved output path on success; surfaces ffmpeg stderr
    /// on failure so the user gets a real error rather than "exit 1".
    pub async fn finish(mut self) -> Result<PathBuf, NarratorError> {
        // Drop stdin first so ffmpeg sees EOF and writes the moov atom.
        if let Some(mut stdin) = self.stdin.take() {
            let _ = stdin.flush().await;
            drop(stdin);
        }
        let status = self
            .child
            .wait()
            .await
            .map_err(|e| NarratorError::FfmpegFailed(format!("encoder wait: {e}")))?;

        let stderr_msg = if let Some(h) = self.stderr_handle.take() {
            h.await.unwrap_or_default()
        } else {
            String::new()
        };

        if !status.success() {
            let tail = stderr_msg
                .lines()
                .rev()
                .take(8)
                .collect::<Vec<_>>()
                .join("\n");
            // Leave `finished = false` so Drop removes the partial file.
            return Err(NarratorError::FfmpegFailed(format!(
                "encoder exited {:?}:\n{tail}",
                status.code()
            )));
        }

        // Atomically move the finished mux onto the real output path. File-over-
        // file rename is atomic on POSIX and NTFS (same guarantee as
        // project_store::atomic_write).
        tokio::fs::rename(&self.partial_path, &self.output_path)
            .await
            .map_err(|e| NarratorError::FfmpegFailed(format!("encoder finalize (rename): {e}")))?;
        self.finished = true;
        Ok(self.output_path.clone())
    }
}
