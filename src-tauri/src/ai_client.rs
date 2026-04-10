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
            .map_err(|e| NarratorError::ApiError(format!("HTTP client error: {e}")))?;

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
                tracing::error!("API error ({status}): {truncated}");
                return Err(NarratorError::ApiError(format!(
                    "Claude API error (HTTP {status}). Check your API key and try again."
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
            .map_err(|e| NarratorError::ApiError(format!("HTTP client error: {e}")))?;

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

        // o3/o4 models require max_completion_tokens; older models use max_tokens
        let is_reasoning_model = self.model.starts_with("o3")
            || self.model.starts_with("o4")
            || self.model.starts_with("o1");
        let token_key = if is_reasoning_model {
            "max_completion_tokens"
        } else {
            "max_tokens"
        };
        let body = json!({
            "model": self.model,
            token_key: 8192,
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
                tracing::error!("API error ({status}): {truncated}");
                return Err(NarratorError::ApiError(format!(
                    "OpenAI API error (HTTP {status}). Check your API key and try again."
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
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
            self.model
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
                .header("x-goog-api-key", &self.api_key)
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
                tracing::error!("API error ({status}): {truncated}");
                return Err(NarratorError::ApiError(format!(
                    "Gemini API error (HTTP {status}). Check your API key and try again."
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
        4. Leave natural gaps (1-3 seconds) between segments for breathing room.\n\
        5. A typical narration covers about 75-85% of the video duration with speech. \
           Aim for MORE narration rather than less — long silent gaps feel empty.\n\
        6. Distribute segments evenly across the full video timeline.\n\
        7. Each segment's text MUST be plain speakable text only. NEVER include \
           markup, tags, or directives like [pause], [break], (pause), etc. \
           The text will be sent directly to a text-to-speech engine.\n\
        8. Write substantive narration for each segment — describe what's happening, \
           explain context, guide the viewer. Avoid very short throwaway phrases.\n\n",
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
    if let Some(last_seg) = script.segments.last() {
        let last_end = last_seg.end_seconds;
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
        } else {
            // Only redistribute if NOT already scaled — avoid double adjustment
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

/// Refine a single narration segment using AI.
/// Takes the segment text, a user instruction, and surrounding context,
/// returns the refined text only (not a full script).
pub async fn refine_segment(
    provider: &dyn AiProvider,
    segment_text: &str,
    instruction: &str,
    context: &str,
) -> Result<String, NarratorError> {
    let system_prompt =
        "You are a professional narration script editor. You will receive a single narration \
        segment and an editing instruction. Apply the instruction to the segment text and return \
        ONLY the refined text. No JSON, no markdown, no explanation — just the new narration text. \
        Preserve any [pause] markers unless the instruction says to remove them.";

    let user_message = json!(format!(
        "Context (surrounding segments for reference, do NOT include them in your output):\n{context}\n\n\
        Segment to refine:\n\"{segment_text}\"\n\n\
        Instruction: {instruction}"
    ));

    let response = provider.generate(system_prompt, user_message).await?;

    // Clean up: remove any accidental quotes, markdown, or explanations
    let refined = response
        .trim()
        .trim_matches('"')
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    if refined.is_empty() {
        return Err(NarratorError::ApiError(
            "AI returned empty refinement".to_string(),
        ));
    }

    Ok(refined.to_string())
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
            let url = "https://generativelanguage.googleapis.com/v1beta/models";
            let resp = client.get(url).header("x-goog-api-key", key).send().await?;

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

    #[test]
    fn test_build_user_message_basic() {
        let metadata = VideoMetadata {
            path: "/tmp/test.mp4".to_string(),
            duration_seconds: 60.0,
            width: 1920,
            height: 1080,
            codec: "h264".to_string(),
            fps: 30.0,
            file_size: 1000000,
        };

        // Call with empty frames
        let result = build_user_message(&[], "Test Video", "A test description", &metadata, "en");
        assert!(result.is_ok());

        let msg = result.unwrap();
        // Should be a JSON array
        assert!(msg.is_array());
        let arr = msg.as_array().unwrap();
        // With no frames, there should be exactly 1 text element (the context)
        assert_eq!(arr.len(), 1);

        // Verify the text content contains key information
        let text = arr[0]["text"].as_str().unwrap();
        assert!(text.contains("Test Video"));
        assert!(text.contains("A test description"));
        assert!(text.contains("60.0"));
        assert!(text.contains("1920x1080"));
        assert!(text.contains("30.0"));
        assert!(text.contains("en"));
        assert!(text.contains("Number of frames: 0"));
    }

    struct MockProvider {
        response: String,
    }

    #[async_trait]
    impl AiProvider for MockProvider {
        async fn generate(
            &self,
            _system_prompt: &str,
            _user_message: serde_json::Value,
        ) -> Result<String, NarratorError> {
            Ok(self.response.clone())
        }
        fn name(&self) -> &str {
            "mock"
        }
        fn model(&self) -> &str {
            "mock-v1"
        }
    }

    #[tokio::test]
    async fn test_generate_narration_parse_valid_json() {
        let valid_response = r#"{
            "title": "Test Narration",
            "total_duration_seconds": 30.0,
            "segments": [
                {
                    "index": 0,
                    "start_seconds": 0.0,
                    "end_seconds": 15.0,
                    "text": "Welcome to the video.",
                    "visual_description": "Opening",
                    "emphasis": [],
                    "pace": "medium",
                    "pause_after_ms": 500,
                    "frame_refs": [0]
                },
                {
                    "index": 1,
                    "start_seconds": 17.0,
                    "end_seconds": 30.0,
                    "text": "Thank you for watching.",
                    "visual_description": "Closing",
                    "emphasis": [],
                    "pace": "slow",
                    "pause_after_ms": 0,
                    "frame_refs": [1]
                }
            ],
            "metadata": {
                "style": "technical",
                "language": "en",
                "provider": "",
                "model": "",
                "generated_at": ""
            }
        }"#;

        let mock = MockProvider {
            response: valid_response.to_string(),
        };

        let result = generate_narration(
            &mock,
            "system prompt",
            json!("user message"),
            "technical",
            "en",
        )
        .await;

        assert!(result.is_ok());
        let script = result.unwrap();
        assert_eq!(script.title, "Test Narration");
        assert_eq!(script.segments.len(), 2);
        assert_eq!(script.segments[0].text, "Welcome to the video.");
        assert_eq!(script.segments[1].text, "Thank you for watching.");
        // Metadata should be filled in from provider since original was empty
        assert_eq!(script.metadata.provider, "mock");
        assert_eq!(script.metadata.model, "mock-v1");
        // generated_at should be filled in since it was empty
        assert!(!script.metadata.generated_at.is_empty());
    }

    #[tokio::test]
    async fn test_generate_narration_parse_invalid_json() {
        let mock = MockProvider {
            response: "this is not valid json at all".to_string(),
        };

        let result = generate_narration(
            &mock,
            "system prompt",
            json!("user message"),
            "technical",
            "en",
        )
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Failed to parse AI response"));
    }

    #[tokio::test]
    async fn test_generate_narration_strips_code_fences() {
        // Some AI providers wrap JSON in markdown code fences
        let actual_response = "```json\n{\"title\":\"Fenced\",\"total_duration_seconds\":10.0,\"segments\":[],\"metadata\":{\"style\":\"test\",\"language\":\"en\",\"provider\":\"mock\",\"model\":\"mock-v1\",\"generated_at\":\"2026-01-01T00:00:00Z\"}}\n```";

        let mock = MockProvider {
            response: actual_response.to_string(),
        };

        let result =
            generate_narration(&mock, "system prompt", json!("user message"), "test", "en").await;

        assert!(result.is_ok());
        let script = result.unwrap();
        assert_eq!(script.title, "Fenced");
    }
}
