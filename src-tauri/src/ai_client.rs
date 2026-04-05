//! Multi-provider AI client supporting Claude and OpenAI for narration generation.

use crate::error::NarratorError;
use crate::models::*;
use crate::video_engine;
use async_trait::async_trait;
use serde_json::json;

#[async_trait]
pub trait AiProvider: Send + Sync {
    async fn generate(
        &self,
        system_prompt: &str,
        user_message: serde_json::Value,
    ) -> Result<String, NarratorError>;

    fn name(&self) -> &str;
    fn model(&self) -> &str;
}

// ── Claude Provider ──

pub struct ClaudeProvider {
    pub api_key: String,
    pub model: String,
    pub temperature: f32,
}

#[async_trait]
impl AiProvider for ClaudeProvider {
    async fn generate(
        &self,
        system_prompt: &str,
        user_message: serde_json::Value,
    ) -> Result<String, NarratorError> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_default();

        let body = json!({
            "model": self.model,
            "max_tokens": 8192,
            "temperature": self.temperature,
            "system": system_prompt,
            "messages": [{
                "role": "user",
                "content": user_message
            }]
        });

        let mut retries = 0;
        let max_retries = 3;

        loop {
            let resp = client
                .post("https://api.anthropic.com/v1/messages")
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await?;

            let status = resp.status();

            if status.is_success() {
                let response_json: serde_json::Value = resp.json().await?;
                let text = response_json["content"]
                    .as_array()
                    .and_then(|blocks| {
                        blocks.iter().find_map(|b| {
                            if b["type"] == "text" {
                                b["text"].as_str().map(|s| s.to_string())
                            } else {
                                None
                            }
                        })
                    })
                    .unwrap_or_default();
                return Ok(text);
            } else if status.as_u16() == 429 || status.as_u16() == 529 {
                retries += 1;
                if retries >= max_retries {
                    return Err(NarratorError::RateLimited);
                }
                let delay = std::time::Duration::from_secs(2u64.pow(retries));
                tokio::time::sleep(delay).await;
            } else {
                let error_text = resp.text().await.unwrap_or_default();
                let truncated = if error_text.len() > 200 {
                    &error_text[..200]
                } else {
                    &error_text
                };
                return Err(NarratorError::ApiError(format!(
                    "Claude API error ({status}): {truncated}"
                )));
            }
        }
    }

    fn name(&self) -> &str {
        "claude"
    }

    fn model(&self) -> &str {
        &self.model
    }
}

// ── OpenAI Provider ──

pub struct OpenAiProvider {
    pub api_key: String,
    pub model: String,
    pub temperature: f32,
}

#[async_trait]
impl AiProvider for OpenAiProvider {
    async fn generate(
        &self,
        system_prompt: &str,
        user_message: serde_json::Value,
    ) -> Result<String, NarratorError> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_default();

        // Convert user_message to OpenAI format
        let user_content = if user_message.is_array() {
            // It's a multimodal content array — convert to OpenAI format
            let parts: Vec<serde_json::Value> = user_message
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .map(|part| {
                    if part["type"] == "image" {
                        // Convert Claude image format to OpenAI
                        let media_type = part["source"]["media_type"]
                            .as_str()
                            .unwrap_or("image/jpeg");
                        let data = part["source"]["data"].as_str().unwrap_or("");
                        json!({
                            "type": "image_url",
                            "image_url": {
                                "url": format!("data:{media_type};base64,{data}")
                            }
                        })
                    } else {
                        // Text part
                        json!({
                            "type": "text",
                            "text": part["text"].as_str().unwrap_or("")
                        })
                    }
                })
                .collect();
            serde_json::Value::Array(parts)
        } else if user_message.is_string() {
            json!([{"type": "text", "text": user_message.as_str().unwrap_or("")}])
        } else {
            json!([{"type": "text", "text": user_message.to_string()}])
        };

        let body = json!({
            "model": self.model,
            "max_tokens": 8192,
            "temperature": self.temperature,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_content}
            ]
        });

        let mut retries = 0;
        let max_retries = 3;

        loop {
            let resp = client
                .post("https://api.openai.com/v1/chat/completions")
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await?;

            let status = resp.status();

            if status.is_success() {
                let response_json: serde_json::Value = resp.json().await?;
                let text = response_json["choices"][0]["message"]["content"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                return Ok(text);
            } else if status.as_u16() == 429 {
                retries += 1;
                if retries >= max_retries {
                    return Err(NarratorError::RateLimited);
                }
                let delay = std::time::Duration::from_secs(2u64.pow(retries));
                tokio::time::sleep(delay).await;
            } else {
                let error_text = resp.text().await.unwrap_or_default();
                let truncated = if error_text.len() > 200 {
                    &error_text[..200]
                } else {
                    &error_text
                };
                return Err(NarratorError::ApiError(format!(
                    "OpenAI API error ({status}): {truncated}"
                )));
            }
        }
    }

    fn name(&self) -> &str {
        "openai"
    }

    fn model(&self) -> &str {
        &self.model
    }
}

// ── Gemini Provider ──

pub struct GeminiProvider {
    pub api_key: String,
    pub model: String,
    pub temperature: f32,
}

#[async_trait]
impl AiProvider for GeminiProvider {
    async fn generate(
        &self,
        system_prompt: &str,
        user_message: serde_json::Value,
    ) -> Result<String, NarratorError> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| NarratorError::ApiError(format!("HTTP client error: {e}")))?;

        // Convert user_message (Claude format) to Gemini parts
        let parts: Vec<serde_json::Value> = if user_message.is_array() {
            user_message
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .map(|part| {
                    if part["type"] == "image" {
                        let media_type = part["source"]["media_type"]
                            .as_str()
                            .unwrap_or("image/jpeg");
                        let data = part["source"]["data"].as_str().unwrap_or("");
                        json!({
                            "inlineData": {
                                "data": data,
                                "mimeType": media_type
                            }
                        })
                    } else {
                        // Text part
                        json!({
                            "text": part["text"].as_str().unwrap_or("")
                        })
                    }
                })
                .collect()
        } else if user_message.is_string() {
            vec![json!({ "text": user_message.as_str().unwrap_or("") })]
        } else {
            vec![json!({ "text": user_message.to_string() })]
        };

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            self.model, self.api_key
        );

        let body = json!({
            "contents": [{ "parts": parts }],
            "systemInstruction": { "parts": [{ "text": system_prompt }] },
            "generationConfig": {
                "temperature": self.temperature,
                "maxOutputTokens": 8192
            }
        });

        let mut retries = 0;
        let max_retries = 3;

        loop {
            let resp = client
                .post(&url)
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await?;

            let status = resp.status();

            if status.is_success() {
                let response_json: serde_json::Value = resp.json().await?;
                let text = response_json["candidates"][0]["content"]["parts"][0]["text"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                return Ok(text);
            } else if status.as_u16() == 429 {
                retries += 1;
                if retries >= max_retries {
                    return Err(NarratorError::RateLimited);
                }
                let delay = std::time::Duration::from_secs(2u64.pow(retries));
                tokio::time::sleep(delay).await;
            } else {
                let error_text = resp.text().await.unwrap_or_default();
                let truncated = if error_text.len() > 200 {
                    &error_text[..200]
                } else {
                    &error_text
                };
                return Err(NarratorError::ApiError(format!(
                    "Gemini API error ({status}): {truncated}"
                )));
            }
        }
    }

    fn name(&self) -> &str {
        "gemini"
    }

    fn model(&self) -> &str {
        &self.model
    }
}

// ── Provider factory ──

pub fn create_provider(config: &AiConfig, api_key: String) -> Box<dyn AiProvider> {
    match config.provider {
        AiProviderKind::Claude => Box::new(ClaudeProvider {
            api_key,
            model: config.model.clone(),
            temperature: config.temperature,
        }),
        AiProviderKind::OpenAi => Box::new(OpenAiProvider {
            api_key,
            model: config.model.clone(),
            temperature: config.temperature,
        }),
        AiProviderKind::Gemini => Box::new(GeminiProvider {
            api_key,
            model: config.model.clone(),
            temperature: config.temperature,
        }),
    }
}

// ── Narration generation ──

pub fn build_system_prompt(
    style: &NarrationStyle,
    context_docs: &[ProcessedDocument],
    custom_prompt: &str,
) -> String {
    let mut prompt = String::new();

    // Base instructions
    prompt.push_str(
        "You are a professional video narrator. Your task is to generate a timed narration \
        script for a video based on the frames and context provided.\n\n\
        You MUST respond with valid JSON matching this exact schema:\n\
        {\n  \"title\": \"string\",\n  \"total_duration_seconds\": number,\n  \
        \"segments\": [\n    {\n      \"index\": number,\n      \"start_seconds\": number,\n      \
        \"end_seconds\": number,\n      \"text\": \"string\",\n      \
        \"visual_description\": \"string\",\n      \"emphasis\": [\"string\"],\n      \
        \"pace\": \"slow\" | \"medium\" | \"fast\",\n      \"pause_after_ms\": number,\n      \
        \"frame_refs\": [number]\n    }\n  ],\n  \"metadata\": {\n    \"style\": \"string\",\n    \
        \"language\": \"string\",\n    \"model\": \"string\",\n    \
        \"generated_at\": \"ISO8601 string\"\n  }\n}\n\n\
        CRITICAL RULES:\n\
        1. Return ONLY the JSON, no markdown code fences, no explanation.\n\
        2. You MUST generate segments covering the ENTIRE video from start to end.\n\
        3. The last segment's end_seconds MUST equal total_duration_seconds.\n\
        4. Segments should NOT be back-to-back. Leave natural gaps (2-5 seconds) between \
           segments where the speaker pauses. Not every second needs narration.\n\
        5. A typical narration covers about 60-70% of the video duration with speech, \
           and 30-40% with silence/pauses between segments.\n\
        6. Distribute segments evenly across the full video timeline.\n\n",
    );

    // Style block
    prompt.push_str("## Narration Style\n\n");
    prompt.push_str(&style.system_prompt);
    prompt.push_str("\n\n");

    // Context documents
    if !context_docs.is_empty() {
        prompt.push_str("## Reference Documents\n\n");
        for doc in context_docs {
            prompt.push_str(&format!(
                "<document name=\"{}\">\n{}\n</document>\n\n",
                doc.name, doc.content
            ));
        }
    }

    // Custom additions
    if !custom_prompt.is_empty() {
        prompt.push_str("## Additional Instructions\n\n");
        prompt.push_str(custom_prompt);
        prompt.push_str("\n\n");
    }

    prompt
}

pub fn build_user_message(
    frames: &[Frame],
    title: &str,
    description: &str,
    video_metadata: &VideoMetadata,
    language: &str,
) -> Result<serde_json::Value, NarratorError> {
    let mut content = Vec::new();

    // Text context — be very explicit about full duration coverage
    let dur = video_metadata.duration_seconds;
    let dur_min = (dur as u64) / 60;
    let dur_sec = (dur as u64) % 60;
    let text_context = format!(
        "Video: \"{title}\"\n\
        Description: {description}\n\
        TOTAL DURATION: {dur:.1}s ({dur_min}m {dur_sec}s)\n\
        Resolution: {}x{} | FPS: {:.1}\n\
        Language: {language}\n\
        Number of frames: {} (sampled evenly across the full {dur:.0}s)\n\n\
        IMPORTANT: Generate narration covering the ENTIRE {dur:.1}s video.\n\
        The LAST segment's end_seconds MUST be {dur:.1}.\n\
        Leave 2-5 second GAPS between segments for natural pacing.\n\
        Distribute narration evenly from 0s to {dur:.0}s — do NOT stop halfway.",
        video_metadata.width,
        video_metadata.height,
        video_metadata.fps,
        frames.len(),
    );

    content.push(json!({
        "type": "text",
        "text": text_context
    }));

    // Add frame images as base64
    for frame in frames {
        if frame.path.exists() {
            let b64 = video_engine::frame_to_base64(&frame.path)?;
            content.push(json!({
                "type": "text",
                "text": format!("[Frame {} at {:.1}s]", frame.index, frame.timestamp_seconds)
            }));
            content.push(json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": "image/jpeg",
                    "data": b64
                }
            }));
        }
    }

    Ok(serde_json::Value::Array(content))
}

pub async fn generate_narration(
    provider: &dyn AiProvider,
    system_prompt: &str,
    user_message: serde_json::Value,
    _style: &str,
    _language: &str,
) -> Result<NarrationScript, NarratorError> {
    let response_text = provider.generate(system_prompt, user_message).await?;

    // Try to parse the JSON response
    // Strip markdown code fences if present
    let json_text = response_text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let mut script: NarrationScript = serde_json::from_str(json_text).map_err(|e| {
        NarratorError::ApiError(format!(
            "Failed to parse AI response as NarrationScript: {e}\nResponse: {json_text}"
        ))
    })?;

    // Fill in metadata that the AI may not have returned correctly
    if script.metadata.provider.is_empty() {
        script.metadata.provider = provider.name().to_string();
    }
    if script.metadata.model.is_empty() || script.metadata.model == "narration_v1" {
        script.metadata.model = provider.model().to_string();
    }
    if script.metadata.generated_at.is_empty() {
        script.metadata.generated_at = chrono::Utc::now().to_rfc3339();
    }

    // Post-process: ensure segments cover the full video duration
    // If AI stopped early, stretch the timeline proportionally
    if !script.segments.is_empty() {
        let last_end = script.segments.last().unwrap().end_seconds;
        let target = script.total_duration_seconds;

        if target > 0.0 && last_end > 0.0 && last_end < target * 0.9 {
            // AI only covered part of the video — scale all timestamps proportionally
            let scale = target / last_end;
            tracing::warn!(
                "Script only covers {:.0}s of {:.0}s video. Scaling timestamps by {:.2}x",
                last_end,
                target,
                scale
            );
            for seg in &mut script.segments {
                seg.start_seconds *= scale;
                seg.end_seconds *= scale;
                // Insert gaps: shrink each segment to ~60% of its slot, leaving 40% as gap
                let slot = seg.end_seconds - seg.start_seconds;
                let speech_portion = slot * 0.65;
                seg.end_seconds = seg.start_seconds + speech_portion;
                if seg.pause_after_ms < 300 {
                    seg.pause_after_ms = ((slot - speech_portion) * 1000.0) as u32;
                }
            }
            // Ensure last segment reaches the end
            if let Some(last) = script.segments.last_mut() {
                last.end_seconds = target;
            }
        }

        // Even if not scaled, ensure gaps exist between contiguous segments
        // If all segments are back-to-back, add gaps
        let all_contiguous = script
            .segments
            .windows(2)
            .all(|w| (w[1].start_seconds - w[0].end_seconds).abs() < 0.1);
        if all_contiguous && script.segments.len() > 1 {
            let total = script.total_duration_seconds;
            let n = script.segments.len() as f64;
            // Redistribute: give each segment a slot of total/n seconds
            // with 65% speech and 35% gap
            let slot = total / n;
            for (i, seg) in script.segments.iter_mut().enumerate() {
                seg.start_seconds = i as f64 * slot;
                seg.end_seconds = seg.start_seconds + slot * 0.65;
                seg.pause_after_ms = (slot * 0.35 * 1000.0) as u32;
            }
            if let Some(last) = script.segments.last_mut() {
                last.end_seconds = total;
                last.pause_after_ms = 0;
            }
        }
    }

    Ok(script)
}

pub async fn translate_script(
    provider: &dyn AiProvider,
    script: &NarrationScript,
    target_language: &str,
) -> Result<NarrationScript, NarratorError> {
    let system_prompt = format!(
        "You are a professional translator. Translate the following timed narration script \
        into {target_language}. Preserve all timestamps, segment boundaries, and [pause] markers. \
        Adapt idioms naturally — do not translate literally. Maintain the same tone and style.\n\n\
        Respond with ONLY valid JSON in the exact same schema as the input. No markdown code fences."
    );

    let script_json = serde_json::to_string_pretty(script)
        .map_err(|e| NarratorError::SerializationError(e.to_string()))?;

    let user_message = json!(script_json);

    let response_text = provider.generate(&system_prompt, user_message).await?;

    let json_text = response_text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let mut translated: NarrationScript = serde_json::from_str(json_text).map_err(|e| {
        NarratorError::ApiError(format!("Failed to parse translation response: {e}"))
    })?;

    // Update metadata
    translated.metadata.language = target_language.to_string();

    Ok(translated)
}

pub async fn validate_api_key(provider: &AiProviderKind, key: &str) -> Result<bool, NarratorError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| NarratorError::ApiError(format!("HTTP client error: {e}")))?;

    match provider {
        AiProviderKind::Claude => {
            let resp = client
                .post("https://api.anthropic.com/v1/messages")
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&json!({
                    "model": "claude-sonnet-4-20250514",
                    "max_tokens": 1,
                    "messages": [{"role": "user", "content": "hi"}]
                }))
                .send()
                .await?;

            // 200 or 400 (bad request but valid key) are both fine
            // 401 means invalid key
            Ok(resp.status().as_u16() != 401)
        }
        AiProviderKind::OpenAi => {
            let resp = client
                .get("https://api.openai.com/v1/models")
                .header("Authorization", format!("Bearer {key}"))
                .send()
                .await?;

            Ok(resp.status().is_success())
        }
        AiProviderKind::Gemini => {
            let url = format!("https://generativelanguage.googleapis.com/v1beta/models?key={key}");
            let resp = client.get(&url).send().await?;

            let status = resp.status().as_u16();
            // 200 = valid key, 400/403 = invalid key
            Ok(status == 200)
        }
    }
}

pub fn get_available_models(provider: &AiProviderKind) -> Vec<String> {
    match provider {
        AiProviderKind::Claude => vec![
            "claude-sonnet-4-20250514".to_string(),
            "claude-opus-4-20250514".to_string(),
        ],
        AiProviderKind::OpenAi => vec!["gpt-4o".to_string(), "o3".to_string()],
        AiProviderKind::Gemini => {
            vec!["gemini-2.5-flash".to_string(), "gemini-2.5-pro".to_string()]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_system_prompt() {
        let style = NarrationStyle {
            id: "technical".to_string(),
            label: "Technical".to_string(),
            description: "Technical deep-dive".to_string(),
            system_prompt: "You are narrating a technical video.".to_string(),
            pacing: "medium".to_string(),
            pause_markers: true,
        };

        let docs = vec![ProcessedDocument {
            name: "glossary.md".to_string(),
            content: "API: Application Programming Interface".to_string(),
            token_estimate: 10,
            source_path: "/tmp/glossary.md".to_string(),
        }];

        let prompt = build_system_prompt(&style, &docs, "Focus on the UI elements.");
        assert!(prompt.contains("technical video"));
        assert!(prompt.contains("glossary.md"));
        assert!(prompt.contains("Focus on the UI elements"));
        assert!(prompt.contains("JSON"));
    }

    #[test]
    fn test_get_available_models() {
        let claude_models = get_available_models(&AiProviderKind::Claude);
        assert_eq!(claude_models.len(), 2);
        assert!(claude_models[0].contains("sonnet"));

        let openai_models = get_available_models(&AiProviderKind::OpenAi);
        assert_eq!(openai_models.len(), 2);
        assert!(openai_models[0].contains("gpt"));

        let gemini_models = get_available_models(&AiProviderKind::Gemini);
        assert_eq!(gemini_models.len(), 2);
        assert!(gemini_models[0].contains("gemini"));
    }

    #[test]
    fn test_create_provider() {
        let config = AiConfig {
            provider: AiProviderKind::Claude,
            model: "claude-sonnet-4-20250514".to_string(),
            temperature: 0.7,
        };
        let provider = create_provider(&config, "test-key".to_string());
        assert_eq!(provider.name(), "claude");
        assert_eq!(provider.model(), "claude-sonnet-4-20250514");
    }

    #[test]
    fn test_create_gemini_provider() {
        let config = AiConfig {
            provider: AiProviderKind::Gemini,
            model: "gemini-2.5-flash".to_string(),
            temperature: 0.7,
        };
        let provider = create_provider(&config, "test-key".to_string());
        assert_eq!(provider.name(), "gemini");
        assert_eq!(provider.model(), "gemini-2.5-flash");
    }
}
