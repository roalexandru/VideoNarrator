use crate::error::NarratorError;
use crate::video_engine;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingConfig {
    pub output_path: String,
    pub screen_index: u32,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub offset_x: u32,
    pub offset_y: u32,
    pub capture_audio: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenDevice {
    pub index: u32,
    pub name: String,
    pub is_screen: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    pub id: u32,
    pub name: String,
    pub owner: String,
}

pub async fn list_windows() -> Result<Vec<WindowInfo>, NarratorError> {
    #[cfg(target_os = "macos")]
    {
        // Use CGWindowListCopyWindowInfo via osascript
        let output = tokio::process::Command::new("osascript")
            .args(["-e", r#"
                set windowList to ""
                tell application "System Events"
                    repeat with proc in (every process whose visible is true)
                        set procName to name of proc
                        repeat with w in (every window of proc)
                            try
                                set windowList to windowList & procName & "|||" & name of w & linefeed
                            end try
                        end repeat
                    end repeat
                end tell
                return windowList
            "#])
            .output().await
            .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut windows = Vec::new();
        for (i, line) in stdout.lines().enumerate() {
            let parts: Vec<&str> = line.split("|||").collect();
            if parts.len() >= 2 {
                windows.push(WindowInfo {
                    id: i as u32,
                    owner: parts[0].trim().to_string(),
                    name: parts[1].trim().to_string(),
                });
            }
        }
        Ok(windows)
    }

    #[cfg(not(target_os = "macos"))]
    {
        Ok(Vec::new())
    }
}

pub async fn list_screens() -> Result<Vec<ScreenDevice>, NarratorError> {
    let ffmpeg = video_engine::detect_ffmpeg()?;

    #[cfg(target_os = "macos")]
    {
        let output = tokio::process::Command::new(ffmpeg.as_os_str())
            .args(["-f", "avfoundation", "-list_devices", "true", "-i", ""])
            .output().await
            .map_err(|e| NarratorError::FfmpegFailed(e.to_string()))?;

        let stderr = String::from_utf8_lossy(&output.stderr);
        let mut devices = Vec::new();

        for line in stderr.lines() {
            // Parse lines like: [avfoundation @ 0x...] [1] Capture screen 0
            if let Some(bracket_start) = line.find('[') {
                let rest = &line[bracket_start + 1..];
                if let Some(bracket_end) = rest.find(']') {
                    if let Ok(idx) = rest[..bracket_end].parse::<u32>() {
                        let name_start = bracket_end + 2;
                        if name_start < rest.len() {
                            let name = rest[name_start..].trim().to_string();
                            let is_screen = name.to_lowercase().contains("screen") || name.to_lowercase().contains("capture");
                            devices.push(ScreenDevice { index: idx, name, is_screen });
                        }
                    }
                }
            }
        }

        // Filter to only screen devices (not cameras)
        let screens: Vec<_> = devices.into_iter().filter(|d| d.is_screen).collect();
        if screens.is_empty() {
            // Fallback: assume screen 1 exists
            return Ok(vec![ScreenDevice { index: 1, name: "Main Screen".into(), is_screen: true }]);
        }
        Ok(screens)
    }

    #[cfg(target_os = "windows")]
    {
        // Windows gdigrab always captures "desktop"
        Ok(vec![ScreenDevice { index: 0, name: "Desktop".into(), is_screen: true }])
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Ok(vec![ScreenDevice { index: 0, name: "Screen :0".into(), is_screen: true }])
    }
}

pub async fn start_recording(
    config: &RecordingConfig,
    stop_flag: Arc<AtomicBool>,
) -> Result<String, NarratorError> {
    let ffmpeg = video_engine::detect_ffmpeg()?;
    let output_path = &config.output_path;

    // Ensure parent dir exists
    if let Some(parent) = PathBuf::from(output_path).parent() {
        std::fs::create_dir_all(parent)?;
    }

    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = tokio::process::Command::new(ffmpeg.as_os_str());
        let input = if config.capture_audio {
            format!("{}:0", config.screen_index)
        } else {
            format!("{}:none", config.screen_index)
        };

        c.args(["-y", "-f", "avfoundation"]);
        c.args(["-capture_cursor", "1"]);
        c.args(["-framerate", &config.fps.to_string()]);
        c.args(["-i", &input]);

        // Crop if region specified
        if config.width > 0 && config.height > 0 {
            c.args(["-vf", &format!("crop={}:{}:{}:{}", config.width, config.height, config.offset_x, config.offset_y)]);
        }

        c.args(["-vcodec", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"]);
        c.arg(output_path);
        c
    };

    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = tokio::process::Command::new(ffmpeg.as_os_str());
        c.args(["-y", "-f", "gdigrab"]);
        c.args(["-framerate", &config.fps.to_string()]);

        if config.width > 0 && config.height > 0 {
            c.args(["-offset_x", &config.offset_x.to_string()]);
            c.args(["-offset_y", &config.offset_y.to_string()]);
            c.args(["-video_size", &format!("{}x{}", config.width, config.height)]);
        }

        c.args(["-i", "desktop"]);
        c.args(["-vcodec", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"]);
        c.arg(output_path);
        c
    };

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut cmd = {
        let mut c = tokio::process::Command::new(ffmpeg.as_os_str());
        c.args(["-y", "-f", "x11grab"]);
        c.args(["-framerate", &config.fps.to_string()]);
        if config.width > 0 && config.height > 0 {
            c.args(["-video_size", &format!("{}x{}", config.width, config.height)]);
        }
        c.args(["-i", &format!(":0.0+{},{}", config.offset_x, config.offset_y)]);
        c.args(["-vcodec", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"]);
        c.arg(output_path);
        c
    };

    // Spawn the recording process
    let mut child = cmd
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| NarratorError::FfmpegFailed(format!("Failed to start recording: {e}")))?;

    // Wait for stop signal, then send 'q' to ffmpeg stdin to stop gracefully
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if stop_flag.load(Ordering::SeqCst) {
                // Send 'q' to ffmpeg to stop recording gracefully
                if let Some(stdin) = child.stdin.as_mut() {
                    use tokio::io::AsyncWriteExt;
                    let _ = stdin.write_all(b"q").await;
                }
                let _ = child.wait().await;
                break;
            }
        }
    });

    Ok(output_path.to_string())
}
