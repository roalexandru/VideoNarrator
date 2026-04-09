//! Built-in text-to-speech using OS native speech engine.
//! macOS: `say` command, Windows: PowerShell SpeechSynthesizer, Linux: espeak.
//! No API key required — works offline.

use crate::error::NarratorError;
use crate::video_engine;
use std::path::Path;
use tokio::process::Command;

/// Generate speech from text using the OS native TTS engine.
/// Outputs an MP3 file at the given path.
pub async fn generate_speech(
    text: &str,
    voice: &str,
    speed: f32,
    output_path: &Path,
) -> Result<(), NarratorError> {
    let ffmpeg = video_engine::detect_ffmpeg()?;

    // Generate WAV/AIFF first using OS command, then convert to MP3

    #[cfg(target_os = "macos")]
    {
        // macOS: use `say` command which outputs to AIFF
        let aiff_path = output_path.with_extension("aiff");
        let mut args = vec!["-o".to_string(), aiff_path.to_string_lossy().to_string()];

        if !voice.is_empty() && voice != "default" {
            args.extend(["-v".to_string(), voice.to_string()]);
        }

        // Speed: `say` uses words per minute, default ~175. Scale relative to 1.0.
        let wpm = (175.0 * speed) as u32;
        args.extend(["-r".to_string(), wpm.to_string()]);

        args.push(text.to_string());

        let output = Command::new("say")
            .args(&args)
            .output()
            .await
            .map_err(|e| NarratorError::FfmpegFailed(format!("macOS say command failed: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(NarratorError::FfmpegFailed(format!("say failed: {stderr}")));
        }

        // Convert AIFF to MP3 via ffmpeg — normalize to 44.1kHz mono for consistency
        let convert = Command::new(ffmpeg.as_os_str())
            .args([
                "-y",
                "-i",
                &aiff_path.to_string_lossy(),
                "-ar",
                "44100",
                "-ac",
                "1",
                "-codec:a",
                "libmp3lame",
                "-q:a",
                "2",
                &output_path.to_string_lossy(),
            ])
            .output()
            .await
            .map_err(|e| NarratorError::FfmpegFailed(format!("ffmpeg convert failed: {e}")))?;

        let _ = std::fs::remove_file(&aiff_path);

        if !convert.status.success() {
            let stderr = String::from_utf8_lossy(&convert.stderr);
            return Err(NarratorError::FfmpegFailed(format!(
                "Audio conversion failed: {stderr}"
            )));
        }
    }

    #[cfg(target_os = "windows")]
    {
        let wav_path = output_path.with_extension("wav");

        // Windows: use PowerShell with SpeechSynthesizer to generate WAV
        let escaped_text = text.replace('\'', "''").replace('"', "`\"");
        let wav_str = wav_path.to_string_lossy().to_string();

        let ps_script = format!(
            r#"Add-Type -AssemblyName System.Speech;
$synth = New-Object System.Speech.Synthesis.SpeechSynthesizer;
$synth.Rate = {rate};
{voice_line}
$synth.SetOutputToWaveFile('{wav}');
$synth.Speak('{text}');
$synth.Dispose();"#,
            rate = ((speed - 1.0) * 5.0).round() as i32, // SAPI rate: -10 to 10, 0 = normal
            voice_line = if !voice.is_empty() && voice != "default" {
                format!("$synth.SelectVoice('{}');", voice)
            } else {
                String::new()
            },
            wav = wav_str.replace('\'', "''"),
            text = escaped_text,
        );

        let output = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &ps_script])
            .output()
            .await
            .map_err(|e| NarratorError::FfmpegFailed(format!("PowerShell TTS failed: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(NarratorError::FfmpegFailed(format!(
                "Windows TTS failed: {stderr}"
            )));
        }

        // Convert WAV to MP3 via ffmpeg
        let convert = Command::new(ffmpeg.as_os_str())
            .args([
                "-y",
                "-i",
                &wav_path.to_string_lossy(),
                "-codec:a",
                "libmp3lame",
                "-q:a",
                "2",
                &output_path.to_string_lossy(),
            ])
            .output()
            .await
            .map_err(|e| NarratorError::FfmpegFailed(format!("ffmpeg convert failed: {e}")))?;

        let _ = std::fs::remove_file(&wav_path);

        if !convert.status.success() {
            let stderr = String::from_utf8_lossy(&convert.stderr);
            return Err(NarratorError::FfmpegFailed(format!(
                "Audio conversion failed: {stderr}"
            )));
        }
    }

    #[cfg(target_os = "linux")]
    {
        let wav_path = output_path.with_extension("wav");

        // Linux: use espeak-ng to generate WAV
        let mut args = vec![
            "--stdout".to_string(),
            "-s".to_string(),
            format!("{}", (175.0 * speed) as u32),
        ];

        if !voice.is_empty() && voice != "default" {
            args.extend(["-v".to_string(), voice.to_string()]);
        }

        args.push(text.to_string());

        let espeak_output = Command::new("espeak-ng")
            .args(&args)
            .output()
            .await
            .or_else(|_| {
                // Fallback to espeak if espeak-ng not available
                std::process::Command::new("espeak").args(&args).output()
            })
            .map_err(|e| NarratorError::FfmpegFailed(format!("espeak failed: {e}")))?;

        if !espeak_output.status.success() {
            return Err(NarratorError::FfmpegFailed("espeak TTS failed".into()));
        }

        // espeak --stdout outputs WAV to stdout, pipe to file
        std::fs::write(&wav_path, &espeak_output.stdout)?;

        let convert = Command::new(ffmpeg.as_os_str())
            .args([
                "-y",
                "-i",
                &wav_path.to_string_lossy(),
                "-codec:a",
                "libmp3lame",
                "-q:a",
                "2",
                &output_path.to_string_lossy(),
            ])
            .output()
            .await
            .map_err(|e| NarratorError::FfmpegFailed(format!("ffmpeg convert failed: {e}")))?;

        let _ = std::fs::remove_file(&wav_path);

        if !convert.status.success() {
            let stderr = String::from_utf8_lossy(&convert.stderr);
            return Err(NarratorError::FfmpegFailed(format!(
                "Audio conversion failed: {stderr}"
            )));
        }
    }

    Ok(())
}

/// List available voices on the current platform.
pub async fn list_voices() -> Result<Vec<BuiltinVoice>, NarratorError> {
    let mut voices = Vec::new();

    #[cfg(target_os = "macos")]
    {
        let output = Command::new("say")
            .args(["-v", "?"])
            .output()
            .await
            .map_err(|e| NarratorError::FfmpegFailed(format!("Failed to list voices: {e}")))?;

        // Novelty/joke voices on macOS that sound terrible for narration
        let novelty: std::collections::HashSet<&str> = [
            "Albert",
            "Bad News",
            "Bahh",
            "Bells",
            "Boing",
            "Bubbles",
            "Cellos",
            "Good News",
            "Jester",
            "Organ",
            "Superstar",
            "Trinoids",
            "Whisper",
            "Wobble",
            "Zarvox",
        ]
        .iter()
        .copied()
        .collect();

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                // Format: "Name              lang_REGION  # description"
                // Name can have spaces (e.g., "Bad News"), so split on the locale pattern
                if let Some(hash_pos) = line.find('#') {
                    let before_hash = line[..hash_pos].trim_end();
                    // Locale is the last whitespace-separated token before #
                    let parts: Vec<&str> = before_hash.split_whitespace().collect();
                    if parts.len() >= 2 {
                        let locale = parts[parts.len() - 1].to_string();
                        let name = parts[..parts.len() - 1].join(" ");
                        if !name.is_empty() && !novelty.contains(name.as_str()) {
                            voices.push(BuiltinVoice {
                                id: name.clone(),
                                name: format!("{} ({})", name, locale.replace('_', "-")),
                                locale: locale.replace('_', "-"),
                            });
                        }
                    }
                }
            }
            // Sort by name
            voices.sort_by(|a, b| a.name.cmp(&b.name));
        }
    }

    #[cfg(target_os = "windows")]
    {
        let output = Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Add-Type -AssemblyName System.Speech; (New-Object System.Speech.Synthesis.SpeechSynthesizer).GetInstalledVoices() | ForEach-Object { $_.VoiceInfo } | Select-Object -Property Name,Culture | ConvertTo-Json",
            ])
            .output()
            .await
            .map_err(|e| NarratorError::FfmpegFailed(format!("Failed to list voices: {e}")))?;

        if output.status.success() {
            let json: serde_json::Value =
                serde_json::from_slice(&output.stdout).unwrap_or_default();
            if let Some(arr) = json.as_array() {
                for v in arr {
                    voices.push(BuiltinVoice {
                        id: v["Name"].as_str().unwrap_or("").to_string(),
                        name: v["Name"].as_str().unwrap_or("").to_string(),
                        locale: v["Culture"].as_str().unwrap_or("en-US").to_string(),
                    });
                }
            } else if json.is_object() {
                // Single voice returns as object, not array
                voices.push(BuiltinVoice {
                    id: json["Name"].as_str().unwrap_or("default").to_string(),
                    name: json["Name"].as_str().unwrap_or("Default").to_string(),
                    locale: json["Culture"].as_str().unwrap_or("en-US").to_string(),
                });
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let output = Command::new("espeak-ng")
            .args(["--voices"])
            .output()
            .await
            .or_else(|_| {
                std::process::Command::new("espeak")
                    .args(["--voices"])
                    .output()
            })
            .ok();

        if let Some(output) = output {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines().skip(1) {
                    // Format: "Pty  Language  Age/Gender  VoiceName  ..."
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 4 {
                        voices.push(BuiltinVoice {
                            id: parts[3].to_string(),
                            name: parts[3].to_string(),
                            locale: parts[1].to_string(),
                        });
                    }
                }
            }
        }
    }

    // Always include a "default" fallback
    if voices.is_empty() {
        voices.push(BuiltinVoice {
            id: "default".to_string(),
            name: "System Default".to_string(),
            locale: "en-US".to_string(),
        });
    }

    Ok(voices)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct BuiltinVoice {
    pub id: String,
    pub name: String,
    pub locale: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_voice_struct_serialization() {
        let voice = BuiltinVoice {
            id: "Samantha".to_string(),
            name: "Samantha".to_string(),
            locale: "en-US".to_string(),
        };

        let json = serde_json::to_string(&voice).expect("serialize");
        let deserialized: BuiltinVoice = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(voice, deserialized);
        assert!(json.contains("Samantha"));
        assert!(json.contains("en-US"));
    }

    #[test]
    fn test_builtin_voice_deserialize_from_json() {
        let json = r#"{"id":"default","name":"System Default","locale":"en-US"}"#;
        let voice: BuiltinVoice = serde_json::from_str(json).expect("deserialize");
        assert_eq!(voice.id, "default");
        assert_eq!(voice.name, "System Default");
        assert_eq!(voice.locale, "en-US");
    }

    #[tokio::test]
    async fn test_list_voices_returns_results() {
        let voices = list_voices().await.expect("list_voices should succeed");
        // Should always return at least the default voice
        assert!(!voices.is_empty());
        // The default fallback voice should be present if no system voices found
        let has_default = voices.iter().any(|v| v.id == "default") || voices.len() > 0;
        assert!(has_default);
    }
}
