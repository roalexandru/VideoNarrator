//! ElevenLabs text-to-speech client for audio narration generation.

use crate::error::NarratorError;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElevenLabsConfig {
    pub api_key: String,
    pub voice_id: String,
    pub model_id: String,
    pub stability: f32,
    pub similarity_boost: f32,
    pub style: f32,
    pub speed: f32,
}

impl Default for ElevenLabsConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            voice_id: "JBFqnCBsd6RMkjVDRZzb".to_string(), // "George" default
            model_id: "eleven_multilingual_v2".to_string(),
            stability: 0.5,
            similarity_boost: 0.75,
            style: 0.0,
            speed: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElevenLabsVoice {
    pub voice_id: String,
    pub name: String,
    pub category: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsResult {
    pub segment_index: usize,
    pub file_path: String,
    pub success: bool,
    pub error: Option<String>,
}

pub async fn list_voices(api_key: &str) -> Result<Vec<ElevenLabsVoice>, NarratorError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| NarratorError::ApiError(format!("HTTP client error: {e}")))?;
    let resp = client
        .get("https://api.elevenlabs.io/v2/voices?page_size=100")
        .header("xi-api-key", api_key)
        .send()
        .await?;

    if resp.status().is_success() {
        let json: serde_json::Value = resp.json().await?;
        let voices: Vec<ElevenLabsVoice> = json["voices"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| {
                        Some(ElevenLabsVoice {
                            voice_id: v["voice_id"].as_str()?.to_string(),
                            name: v["name"].as_str()?.to_string(),
                            category: v["category"].as_str().unwrap_or("unknown").to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        if !voices.is_empty() {
            return Ok(voices);
        }
    }

    // Fallback: return popular default voices if API listing fails (missing permissions)
    tracing::warn!("Could not list ElevenLabs voices, using defaults");
    Ok(default_voices())
}

pub fn default_voices() -> Vec<ElevenLabsVoice> {
    vec![
        ElevenLabsVoice {
            voice_id: "JBFqnCBsd6RMkjVDRZzb".into(),
            name: "George".into(),
            category: "premade".into(),
        },
        ElevenLabsVoice {
            voice_id: "onwK4e9ZLuTAKqWW03F9".into(),
            name: "Daniel".into(),
            category: "premade".into(),
        },
        ElevenLabsVoice {
            voice_id: "EXAVITQu4vr4xnSDxMaL".into(),
            name: "Sarah".into(),
            category: "premade".into(),
        },
        ElevenLabsVoice {
            voice_id: "FGY2WhTYpPnrIDTdsKH5".into(),
            name: "Laura".into(),
            category: "premade".into(),
        },
        ElevenLabsVoice {
            voice_id: "IKne3meq5aSn9XLyUdCD".into(),
            name: "Charlie".into(),
            category: "premade".into(),
        },
        ElevenLabsVoice {
            voice_id: "TX3LPaxmHKxFdv7VOQHJ".into(),
            name: "Liam".into(),
            category: "premade".into(),
        },
        ElevenLabsVoice {
            voice_id: "pFZP5JQG7iQjIQuC4Bku".into(),
            name: "Lily".into(),
            category: "premade".into(),
        },
        ElevenLabsVoice {
            voice_id: "bIHbv24MWmeRgasZH58o".into(),
            name: "Will".into(),
            category: "premade".into(),
        },
        ElevenLabsVoice {
            voice_id: "nPczCjzI2devNBz1zQrb".into(),
            name: "Brian".into(),
            category: "premade".into(),
        },
        ElevenLabsVoice {
            voice_id: "XB0fDUnXU5powFXDhCwa".into(),
            name: "Charlotte".into(),
            category: "premade".into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_voices_not_empty() {
        let voices = default_voices();
        assert!(!voices.is_empty());
        // Should have a reasonable number of default voices
        assert!(voices.len() >= 5);
    }

    #[test]
    fn test_default_voices_have_ids() {
        let voices = default_voices();
        for voice in &voices {
            assert!(
                !voice.voice_id.is_empty(),
                "Voice has empty voice_id: {:?}",
                voice.name
            );
            assert!(
                !voice.name.is_empty(),
                "Voice has empty name for id: {}",
                voice.voice_id
            );
            assert!(
                !voice.category.is_empty(),
                "Voice has empty category: {} ({})",
                voice.name,
                voice.voice_id
            );
        }
    }

    #[test]
    fn test_default_config_voice_is_in_default_voices() {
        let config = ElevenLabsConfig::default();
        let voices = default_voices();
        assert!(
            voices.iter().any(|v| v.voice_id == config.voice_id),
            "Default config voice_id '{}' not found in default voices list",
            config.voice_id
        );
    }

    #[test]
    fn test_default_voices_unique_ids() {
        let voices = default_voices();
        let ids: Vec<&str> = voices.iter().map(|v| v.voice_id.as_str()).collect();
        let unique: std::collections::HashSet<&str> = ids.iter().copied().collect();
        assert_eq!(ids.len(), unique.len(), "Duplicate voice IDs found");
    }
}

pub async fn generate_speech(
    config: &ElevenLabsConfig,
    text: &str,
    output_path: &PathBuf,
) -> Result<(), NarratorError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| NarratorError::ApiError(format!("HTTP client error: {e}")))?;
    let url = format!(
        "https://api.elevenlabs.io/v1/text-to-speech/{}?output_format=mp3_44100_128",
        config.voice_id
    );

    let body = serde_json::json!({
        "text": text,
        "model_id": config.model_id,
        "voice_settings": {
            "stability": config.stability,
            "similarity_boost": config.similarity_boost,
            "style": config.style,
            "speed": config.speed,
        }
    });

    let mut retries = 0;
    loop {
        let resp = client
            .post(&url)
            .header("xi-api-key", &config.api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if status.is_success() {
            let bytes = resp.bytes().await?;
            std::fs::write(output_path, &bytes)?;
            return Ok(());
        } else if status.as_u16() == 429 {
            retries += 1;
            if retries >= 3 {
                return Err(NarratorError::RateLimited);
            }
            tokio::time::sleep(std::time::Duration::from_secs(2u64.pow(retries))).await;
        } else {
            let text = resp.text().await.unwrap_or_default();
            return Err(NarratorError::ApiError(format!(
                "ElevenLabs TTS error ({status}): {text}"
            )));
        }
    }
}

pub async fn validate_key(api_key: &str) -> Result<bool, NarratorError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| NarratorError::ApiError(format!("HTTP client error: {e}")))?;
    let key = api_key.trim();

    // Test with a minimal TTS request — this works even with restricted keys
    // We send an empty-ish request that will fail with 422 (valid key) or 401 (invalid key)
    let resp = client
        .post("https://api.elevenlabs.io/v1/text-to-speech/JBFqnCBsd6RMkjVDRZzb")
        .header("xi-api-key", key)
        .header("Content-Type", "application/json")
        .body("{}")
        .send()
        .await
        .map_err(|e| NarratorError::ApiError(format!("Failed to connect to ElevenLabs: {e}")))?;

    let status = resp.status().as_u16();
    tracing::info!("ElevenLabs key validation: HTTP {status}");

    // 401 = bad key. Anything else (200, 422, 400) = key is valid
    Ok(status != 401)
}
