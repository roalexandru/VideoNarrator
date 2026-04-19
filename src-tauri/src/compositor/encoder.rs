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
use crate::process_utils::CommandNoWindow;
use crate::video_engine;

pub struct Encoder {
    child: Child,
    stdin: Option<ChildStdin>,
    stderr_handle: Option<tokio::task::JoinHandle<String>>,
    output_path: PathBuf,
}

impl Encoder {
    /// Start an encoder writing to `output_path`. If `audio_source` is
    /// supplied, its first audio stream is muxed (`-c:a copy`) into the
    /// output; if it is `None`, the output has no audio track.
    pub async fn start(
        output_path: &Path,
        width: u32,
        height: u32,
        fps: f64,
        audio_source: Option<&Path>,
    ) -> Result<Self, NarratorError> {
        Self::start_inner(output_path, width, height, fps, audio_source, "copy").await
    }

    /// Same as `start` but re-encodes the audio source to AAC.
    /// Used by the single-pass pipeline where the audio source is a PCM WAV
    /// (timeline-assembled) that needs AAC for the MP4 container.
    pub async fn start_with_aac(
        output_path: &Path,
        width: u32,
        height: u32,
        fps: f64,
        audio_source: Option<&Path>,
    ) -> Result<Self, NarratorError> {
        Self::start_inner(output_path, width, height, fps, audio_source, "aac").await
    }

    async fn start_inner(
        output_path: &Path,
        width: u32,
        height: u32,
        fps: f64,
        audio_source: Option<&Path>,
        audio_codec: &str,
    ) -> Result<Self, NarratorError> {
        let ffmpeg = video_engine::detect_ffmpeg()?;
        let size_arg = format!("{width}x{height}");
        let fps_arg = format!("{:.6}", fps);

        let mut cmd = tokio::process::Command::new(ffmpeg.as_os_str());
        cmd.no_window();
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
                "-preset",
                "ultrafast",
                "-crf",
                "0",
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
                "-preset",
                "ultrafast",
                "-crf",
                "0",
                "-pix_fmt",
                "yuv420p",
                "-an",
                "-movflags",
                "+faststart",
            ]);
        }

        cmd.arg(output_path.as_os_str())
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
            return Err(NarratorError::FfmpegFailed(format!(
                "encoder exited {:?}:\n{tail}",
                status.code()
            )));
        }
        Ok(self.output_path)
    }
}
