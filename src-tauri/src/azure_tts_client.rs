//! Azure Cognitive Services text-to-speech client for audio narration generation.

use crate::error::NarratorError;
use crate::http_client;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AzureTtsConfig {
    pub api_key: String,
    pub region: String,
    pub voice_name: String,
    pub speaking_style: String,
    pub speed: f32,
}

impl Default for AzureTtsConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            region: "eastus".to_string(),
            voice_name: "en-US-JennyNeural".to_string(),
            speaking_style: "narration-professional".to_string(),
            speed: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AzureTtsVoice {
    pub short_name: String,
    pub display_name: String,
    pub locale: String,
    pub gender: String,
}

pub async fn list_voices(api_key: &str, region: &str) -> Result<Vec<AzureTtsVoice>, NarratorError> {
    let client = http_client::shared();

    let url = format!(
        "https://{}.tts.speech.microsoft.com/cognitiveservices/voices/list",
        region
    );

    let resp = client
        .get(&url)
        .header("Ocp-Apim-Subscription-Key", api_key)
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => {
            let json: serde_json::Value = r.json().await?;
            let voices: Vec<AzureTtsVoice> = json
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| {
                            Some(AzureTtsVoice {
                                short_name: v["ShortName"].as_str()?.to_string(),
                                display_name: v["DisplayName"].as_str()?.to_string(),
                                locale: v["Locale"].as_str()?.to_string(),
                                gender: v["Gender"].as_str()?.to_string(),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();

            if !voices.is_empty() {
                return Ok(voices);
            }

            tracing::warn!("Azure voices list was empty, using defaults");
            Ok(default_voices())
        }
        Ok(r) => {
            tracing::warn!(
                "Azure voices list failed with status {}, using defaults",
                r.status()
            );
            Ok(default_voices())
        }
        Err(e) => {
            tracing::warn!("Azure voices list request failed: {e}, using defaults");
            Ok(default_voices())
        }
    }
}

pub fn default_voices() -> Vec<AzureTtsVoice> {
    vec![
        AzureTtsVoice {
            short_name: "en-US-JennyNeural".into(),
            display_name: "Jenny".into(),
            locale: "en-US".into(),
            gender: "Female".into(),
        },
        AzureTtsVoice {
            short_name: "en-US-GuyNeural".into(),
            display_name: "Guy".into(),
            locale: "en-US".into(),
            gender: "Male".into(),
        },
        AzureTtsVoice {
            short_name: "ja-JP-NanamiNeural".into(),
            display_name: "Nanami".into(),
            locale: "ja-JP".into(),
            gender: "Female".into(),
        },
        AzureTtsVoice {
            short_name: "ja-JP-KeitaNeural".into(),
            display_name: "Keita".into(),
            locale: "ja-JP".into(),
            gender: "Male".into(),
        },
        AzureTtsVoice {
            short_name: "de-DE-KatjaNeural".into(),
            display_name: "Katja".into(),
            locale: "de-DE".into(),
            gender: "Female".into(),
        },
        AzureTtsVoice {
            short_name: "de-DE-ConradNeural".into(),
            display_name: "Conrad".into(),
            locale: "de-DE".into(),
            gender: "Male".into(),
        },
        AzureTtsVoice {
            short_name: "fr-FR-DeniseNeural".into(),
            display_name: "Denise".into(),
            locale: "fr-FR".into(),
            gender: "Female".into(),
        },
        AzureTtsVoice {
            short_name: "fr-FR-HenriNeural".into(),
            display_name: "Henri".into(),
            locale: "fr-FR".into(),
            gender: "Male".into(),
        },
        AzureTtsVoice {
            short_name: "pt-BR-FranciscaNeural".into(),
            display_name: "Francisca".into(),
            locale: "pt-BR".into(),
            gender: "Female".into(),
        },
        AzureTtsVoice {
            short_name: "pt-BR-AntonioNeural".into(),
            display_name: "Antonio".into(),
            locale: "pt-BR".into(),
            gender: "Male".into(),
        },
    ]
}

/// XML-escape text for safe inclusion in SSML content.
fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Strip any XML/SSML-like tags from text to prevent injection.
fn strip_ssml_tags(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut in_tag = false;
    for ch in text.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
            }
            _ if !in_tag => result.push(ch),
            _ => {} // skip chars inside tags
        }
    }
    result
}

pub async fn generate_speech(
    config: &AzureTtsConfig,
    text: &str,
    output_path: &PathBuf,
) -> Result<(), NarratorError> {
    let client = http_client::shared();

    let url = format!(
        "https://{}.tts.speech.microsoft.com/cognitiveservices/v1",
        config.region
    );

    let escaped_text = xml_escape(&strip_ssml_tags(text));
    let escaped_voice = xml_escape(&config.voice_name);
    let escaped_style = xml_escape(&config.speaking_style);
    // Azure TTS rate: relative percentage (e.g. "+0%", "+50%", "-20%") or multiplier number
    let speed_str = format!("{:+.0}%", (config.speed - 1.0) * 100.0);

    // Build inner content: optionally wrap with express-as
    let inner_content = format!("<prosody rate='{}'>{}</prosody>", speed_str, escaped_text);

    let voice_content = if config.speaking_style.is_empty() || config.speaking_style == "general" {
        inner_content
    } else {
        format!(
            "<mstts:express-as style='{}'>{}</mstts:express-as>",
            escaped_style, inner_content
        )
    };

    let ssml = format!(
        "<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' \
         xmlns:mstts='http://www.w3.org/2001/mstts' xml:lang='en-US'>\
         <voice name='{}'>{}</voice></speak>",
        escaped_voice, voice_content
    );

    let mut retries = 0;
    loop {
        let resp = client
            .post(&url)
            .header("Ocp-Apim-Subscription-Key", &config.api_key)
            .header("Content-Type", "application/ssml+xml")
            .header(
                "X-Microsoft-OutputFormat",
                "audio-24khz-160kbitrate-mono-mp3",
            )
            .header("User-Agent", "Narrator")
            .body(ssml.clone())
            .send()
            .await?;

        let status = resp.status();
        if status.is_success() {
            let bytes = resp.bytes().await?;
            tokio::fs::write(output_path, &bytes).await?;
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
                "Azure TTS error ({status}): {text}"
            )));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xml_escape() {
        assert_eq!(xml_escape("hello"), "hello");
        assert_eq!(xml_escape("a & b"), "a &amp; b");
        assert_eq!(xml_escape("<script>"), "&lt;script&gt;");
        assert_eq!(xml_escape("say \"hi\""), "say &quot;hi&quot;");
        assert_eq!(xml_escape("it's"), "it&apos;s");
        // Multiple entities in one string
        assert_eq!(
            xml_escape("<a & b \"c\" 'd'>"),
            "&lt;a &amp; b &quot;c&quot; &apos;d&apos;&gt;"
        );
        // Empty string
        assert_eq!(xml_escape(""), "");
    }

    #[test]
    fn test_strip_ssml_tags() {
        assert_eq!(strip_ssml_tags("hello"), "hello");
        assert_eq!(strip_ssml_tags("<b>bold</b>"), "bold");
        assert_eq!(
            strip_ssml_tags("<speak>Hello <break/>world</speak>"),
            "Hello world"
        );
        // Nested tags
        assert_eq!(
            strip_ssml_tags("<prosody rate='fast'><emphasis>text</emphasis></prosody>"),
            "text"
        );
        // No tags
        assert_eq!(strip_ssml_tags("plain text"), "plain text");
        // Empty string
        assert_eq!(strip_ssml_tags(""), "");
        // Self-closing tag
        assert_eq!(strip_ssml_tags("before<br/>after"), "beforeafter");
    }

    #[test]
    fn test_default_voices() {
        let voices = default_voices();
        assert!(!voices.is_empty());
        // Should have at least 2 voices (male + female for English)
        assert!(voices.len() >= 2);

        // Verify structure of each voice
        for voice in &voices {
            assert!(!voice.short_name.is_empty());
            assert!(!voice.display_name.is_empty());
            assert!(!voice.locale.is_empty());
            assert!(
                voice.gender == "Male" || voice.gender == "Female",
                "Unexpected gender: {}",
                voice.gender
            );
        }

        // Verify that the default voice (Jenny) is present
        assert!(voices.iter().any(|v| v.short_name == "en-US-JennyNeural"));
    }
}

pub async fn validate_key(api_key: &str, region: &str) -> Result<bool, NarratorError> {
    let client = http_client::shared();

    let key = api_key.trim();
    let url = format!(
        "https://{}.tts.speech.microsoft.com/cognitiveservices/voices/list",
        region
    );

    let resp = client
        .get(&url)
        .header("Ocp-Apim-Subscription-Key", key)
        .send()
        .await
        .map_err(|e| NarratorError::ApiError(format!("Failed to connect to Azure TTS: {e}")))?;

    let status = resp.status().as_u16();
    tracing::info!("Azure TTS key validation: HTTP {status}");

    // 200 = valid key + valid region. 401 = bad key. Other statuses are ambiguous.
    Ok(status == 200)
}
