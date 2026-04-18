//! Multi-provider AI client supporting Claude and OpenAI for narration generation.

use crate::error::NarratorError;
use crate::http_client;
use crate::models::*;
use crate::video_engine;
use async_trait::async_trait;
use serde_json::json;

/// Truncate a string to at most `max_chars` CHARACTERS (not bytes), safe for
/// multi-byte UTF-8 text like Japanese or emoji. Returns a borrowed slice when
/// the string already fits, otherwise an owned String.
fn truncate_chars(s: &str, max_chars: usize) -> std::borrow::Cow<'_, str> {
    if s.chars().count() <= max_chars {
        std::borrow::Cow::Borrowed(s)
    } else {
        std::borrow::Cow::Owned(s.chars().take(max_chars).collect())
    }
}

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
        let client = http_client::shared();

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
                let truncated = truncate_chars(&error_text, 200);
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
        let client = http_client::shared();

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
                let truncated = truncate_chars(&error_text, 200);
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
        let client = http_client::shared();

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
                let truncated = truncate_chars(&error_text, 200);
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
        Distribute narration evenly from 0s to {dur:.0}s — do NOT stop halfway.\n\n\
        PAY CLOSE ATTENTION to what is visible on screen in each frame:\n\
        - Read any text visible in terminals, code editors, browsers, or dialogs\n\
        - Note the state of visible applications (what window is active, what buttons are shown)\n\
        - Describe what is happening based on the visible UI state changes between frames\n\
        - Reference specific on-screen content in the narration (commands typed, output shown, menus opened)",
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

/// Generate narration in chunks when there are too many frames for a single API call.
/// Splits frames into batches, generates segments per batch with context from previous.
async fn generate_chunked(
    provider: &dyn AiProvider,
    system_prompt: &str,
    user_message: &serde_json::Value,
    image_count: usize,
) -> Result<String, NarratorError> {
    let parts = user_message
        .as_array()
        .ok_or_else(|| NarratorError::ApiError("Expected array for chunked generation".into()))?;

    // Separate text parts (first) from image+text pairs
    let mut text_parts = Vec::new();
    let mut image_pairs: Vec<(serde_json::Value, serde_json::Value)> = Vec::new(); // (text label, image)

    let mut i = 0;
    while i < parts.len() {
        if parts[i]["type"] == "image" {
            // This shouldn't happen — text label comes before image
            image_pairs.push((json!({"type": "text", "text": ""}), parts[i].clone()));
            i += 1;
        } else if i + 1 < parts.len() && parts[i + 1]["type"] == "image" {
            // Text label + image pair
            image_pairs.push((parts[i].clone(), parts[i + 1].clone()));
            i += 2;
        } else {
            // Text-only part (context, instructions)
            text_parts.push(parts[i].clone());
            i += 1;
        }
    }

    let num_chunks = image_pairs.len().div_ceil(MAX_FRAMES_PER_CALL);
    tracing::info!(
        "Chunked generation: {} frames in {} chunks of up to {}",
        image_count,
        num_chunks,
        MAX_FRAMES_PER_CALL
    );

    // Extract frame timestamps from labels so we can compute per-chunk time bounds.
    // Collect timestamps aligned to image_pairs order.
    let frame_times: Vec<f64> = image_pairs
        .iter()
        .map(|(label, _img)| {
            let text = label.get("text").and_then(|v| v.as_str()).unwrap_or("");
            // Parse "[Frame N at X.Xs]"
            text.find(" at ")
                .and_then(|idx| {
                    let after = &text[idx + 4..];
                    after.find('s').and_then(|s| after[..s].parse::<f64>().ok())
                })
                .unwrap_or(0.0)
        })
        .collect();

    let mut all_segments: Vec<crate::models::Segment> = Vec::new();
    let mut merged_script: Option<NarrationScript> = None;

    for chunk_idx in 0..num_chunks {
        let start = chunk_idx * MAX_FRAMES_PER_CALL;
        let end = (start + MAX_FRAMES_PER_CALL).min(image_pairs.len());
        let chunk_pairs = &image_pairs[start..end];

        // Compute time bounds for this chunk from frame timestamps.
        // chunk_start = first frame's timestamp (or previous chunk's end if context exists)
        // chunk_end   = next chunk's first frame timestamp (or total video duration if last chunk)
        let chunk_first_ts = frame_times.get(start).copied().unwrap_or(0.0);
        let chunk_last_ts = frame_times.get(end - 1).copied().unwrap_or(chunk_first_ts);
        let next_chunk_first_ts = frame_times.get(end).copied();

        // Bound the chunk strictly between the first frame and the first frame of the next chunk.
        // For the final chunk, allow up to chunk_last_ts + buffer (no hard upper bound known here).
        let chunk_start_time = if chunk_idx == 0 {
            0.0
        } else {
            // Hard-lock: must start where previous chunk ended
            all_segments
                .last()
                .map(|s| s.end_seconds)
                .unwrap_or(chunk_first_ts)
        };
        let chunk_end_time = next_chunk_first_ts.unwrap_or(chunk_last_ts + 30.0);

        // Build the message for this chunk
        let mut chunk_content = text_parts.clone();

        // Add context from previous chunks + strict time-bound instructions
        if !all_segments.is_empty() {
            let prev_summary: String = all_segments
                .iter()
                .rev()
                .take(5) // only the last 5 to keep prompt tight
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .map(|s| {
                    format!(
                        "[{:.2}s-{:.2}s]: {}",
                        s.start_seconds,
                        s.end_seconds,
                        truncate_chars(&s.text, 80)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            chunk_content.push(json!({
                "type": "text",
                "text": format!(
                    "\n--- PREVIOUSLY GENERATED SEGMENTS (for context only — DO NOT repeat or overlap them) ---\n{prev_summary}\n\n\
                    --- STRICT TIME BOUNDS for this batch ---\n\
                    All new segments in this batch MUST have start_seconds >= {:.2} and end_seconds <= {:.2}.\n\
                    The first new segment's start_seconds MUST equal {:.2} (continuation from previous batch).\n\
                    Segments MUST be in strictly ascending time order. DO NOT emit any segment that overlaps the previous batch.\n\
                    --- NOW generate narration for the following frames within these time bounds. ---\n",
                    chunk_start_time, chunk_end_time, chunk_start_time
                )
            }));
        } else {
            chunk_content.push(json!({
                "type": "text",
                "text": format!(
                    "\nThis is batch {}/{num_chunks} of frames.\n\
                    --- STRICT TIME BOUNDS for this batch ---\n\
                    All segments MUST have start_seconds >= {:.2} and end_seconds <= {:.2}.\n\
                    Segments MUST be in strictly ascending time order.\n\
                    --- Generate narration segments for these frames within these bounds. ---\n",
                    chunk_idx + 1, chunk_start_time, chunk_end_time
                )
            }));
        }

        // Add the frame images for this chunk
        for (text_label, image) in chunk_pairs {
            chunk_content.push(text_label.clone());
            chunk_content.push(image.clone());
        }

        let chunk_message = serde_json::Value::Array(chunk_content);

        tracing::info!(
            "Chunk {}/{}: {} frames ({:.2}s → {:.2}s)",
            chunk_idx + 1,
            num_chunks,
            end - start,
            chunk_start_time,
            chunk_end_time
        );

        let response = generate_with_retry(provider, system_prompt, chunk_message).await?;

        // Parse the chunk response
        let json_text = response
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        let chunk_script: NarrationScript = serde_json::from_str(json_text).map_err(|e| {
            NarratorError::ApiError(format!(
                "Failed to parse chunk {} response: {e}\nResponse: {}",
                chunk_idx + 1,
                truncate_chars(json_text, 500)
            ))
        })?;

        if merged_script.is_none() {
            let base = NarrationScript {
                title: chunk_script.title.clone(),
                total_duration_seconds: chunk_script.total_duration_seconds,
                segments: Vec::new(),
                metadata: chunk_script.metadata.clone(),
            };
            merged_script = Some(base);
        }

        // Clamp segments to this chunk's time bounds and drop any that violate ordering
        // relative to segments already accumulated.
        let clamped = clamp_chunk_segments(chunk_script.segments, chunk_start_time, chunk_end_time);
        let last_end = all_segments.last().map(|s| s.end_seconds).unwrap_or(0.0);
        let mut kept = 0usize;
        let mut skipped = 0usize;
        for mut seg in clamped {
            // Hard-lock: reject any segment that starts before the previous one ended.
            if seg.start_seconds < last_end - 0.01 {
                // Try to rescue by pushing forward if the segment still has room
                if seg.end_seconds > last_end + 0.3 {
                    seg.start_seconds = last_end;
                } else {
                    skipped += 1;
                    continue;
                }
            }
            all_segments.push(seg);
            kept += 1;
        }
        if skipped > 0 {
            tracing::warn!(
                "Chunk {}: kept {} segments, skipped {} that violated time bounds",
                chunk_idx + 1,
                kept,
                skipped
            );
        }
    }

    // Build the final merged script
    if let Some(mut script) = merged_script {
        // Final normalization pass (guarantees monotonic, non-overlapping, re-indexed)
        let normalized = normalize_timeline(all_segments, script.total_duration_seconds);
        script.segments = normalized;
        // Return as JSON string (same format as single-call response)
        serde_json::to_string(&script)
            .map_err(|e| NarratorError::ApiError(format!("Failed to serialize merged script: {e}")))
    } else {
        Err(NarratorError::ApiError("No chunks generated".into()))
    }
}

/// Clamp segments to a chunk's time range and drop invalid ones.
fn clamp_chunk_segments(
    segments: Vec<Segment>,
    chunk_start: f64,
    chunk_end: f64,
) -> Vec<Segment> {
    segments
        .into_iter()
        .filter(|s| s.start_seconds.is_finite() && s.end_seconds.is_finite())
        .filter(|s| !s.text.trim().is_empty())
        .map(|mut s| {
            s.start_seconds = s.start_seconds.max(chunk_start);
            s.end_seconds = s.end_seconds.min(chunk_end);
            s
        })
        .filter(|s| s.end_seconds > s.start_seconds + 0.3)
        .collect()
}

/// Normalize a timeline of segments: filter malformed, sort, dedupe, resolve overlaps.
/// This is the defensive last-line post-processor that guarantees monotonic timestamps.
pub fn normalize_timeline(
    mut segments: Vec<Segment>,
    video_duration: f64,
) -> Vec<Segment> {
    let original_len = segments.len();

    // 1. Filter obviously malformed
    segments.retain(|s| {
        s.start_seconds.is_finite()
            && s.end_seconds.is_finite()
            && s.start_seconds >= 0.0
            && s.start_seconds < video_duration + 1.0
            && s.end_seconds > s.start_seconds
            && !s.text.trim().is_empty()
    });

    // 2. Clamp end times to video duration
    for s in segments.iter_mut() {
        s.end_seconds = s.end_seconds.min(video_duration);
    }

    // 3. Sort by start time (primary), end time (secondary)
    segments.sort_by(|a, b| {
        a.start_seconds
            .partial_cmp(&b.start_seconds)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(
                a.end_seconds
                    .partial_cmp(&b.end_seconds)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });

    // 4. Deduplicate segments with nearly identical start/end
    segments.dedup_by(|a, b| {
        (a.start_seconds - b.start_seconds).abs() < 0.2
            && (a.end_seconds - b.end_seconds).abs() < 0.2
    });

    // 5. Resolve overlaps
    let mut fixed: Vec<Segment> = Vec::with_capacity(segments.len());
    let mut dropped_count = 0usize;
    let mut clamped_count = 0usize;
    for seg in segments {
        if let Some(last) = fixed.last_mut() {
            if seg.start_seconds < last.end_seconds {
                let overlap = last.end_seconds - seg.start_seconds;
                let seg_len = seg.end_seconds - seg.start_seconds;
                if seg.end_seconds <= last.end_seconds {
                    // Fully contained — drop it
                    tracing::warn!(
                        "Dropping segment fully contained in previous [{:.2}-{:.2}]: \"{}\"",
                        seg.start_seconds,
                        seg.end_seconds,
                        truncate_chars(&seg.text, 60)
                    );
                    dropped_count += 1;
                    continue;
                } else if overlap > seg_len * 0.5 {
                    // Heavy overlap: truncate previous to make room
                    tracing::warn!(
                        "Heavy overlap: truncating previous [{:.2}-{:.2}] to [{:.2}]",
                        last.start_seconds,
                        last.end_seconds,
                        seg.start_seconds
                    );
                    last.end_seconds = seg.start_seconds;
                    clamped_count += 1;
                } else {
                    // Light overlap: push new segment's start
                    let mut s = seg;
                    let old_start = s.start_seconds;
                    s.start_seconds = last.end_seconds;
                    tracing::warn!(
                        "Light overlap: pushed segment start {:.2} → {:.2}",
                        old_start,
                        s.start_seconds
                    );
                    if s.end_seconds - s.start_seconds >= 0.3 {
                        fixed.push(s);
                    } else {
                        dropped_count += 1;
                    }
                    clamped_count += 1;
                    continue;
                }
            }
        }
        fixed.push(seg);
    }

    // 6. Ensure minimum 0.5s duration per segment
    for s in fixed.iter_mut() {
        if s.end_seconds - s.start_seconds < 0.5 {
            s.end_seconds = s.start_seconds + 0.5;
        }
    }

    // 7. Final sanity pass: ensure strict ascending order
    for i in 1..fixed.len() {
        if fixed[i].start_seconds < fixed[i - 1].end_seconds {
            fixed[i].start_seconds = fixed[i - 1].end_seconds;
            if fixed[i].end_seconds <= fixed[i].start_seconds {
                fixed[i].end_seconds = fixed[i].start_seconds + 0.5;
            }
        }
    }

    // 8. Re-index
    for (i, s) in fixed.iter_mut().enumerate() {
        s.index = i;
    }

    if original_len != fixed.len() || dropped_count > 0 || clamped_count > 0 {
        tracing::info!(
            "normalize_timeline: {} → {} segments ({} dropped, {} clamped)",
            original_len,
            fixed.len(),
            dropped_count,
            clamped_count
        );
    }

    fixed
}

/// Merge adjacent segments that are shorter than a natural-speech floor.
///
/// Background: the AI sometimes returns many very-short segments (0.5-1.0s
/// each) which are below what a human can naturally speak and produce
/// unnaturally choppy narration. TTS for a 0.5s slot with 10 words of text
/// either speeds up unnaturally or overruns the slot, desynchronizing audio.
///
/// This algorithmic post-pass walks the script and merges any segment whose
/// duration falls below `min_duration` into its neighbor, preferring the
/// next segment (so the timeline extends forward rather than backward).
/// Runs AFTER `normalize_timeline` which already guarantees monotonic,
/// non-overlapping segments.
pub fn merge_short_segments(
    segments: Vec<Segment>,
    min_duration: f64,
) -> Vec<Segment> {
    if segments.len() < 2 {
        return segments;
    }

    let mut out: Vec<Segment> = Vec::with_capacity(segments.len());
    let mut merged_count = 0usize;

    for seg in segments {
        let seg_dur = seg.end_seconds - seg.start_seconds;
        if seg_dur < min_duration {
            if let Some(last) = out.last_mut() {
                // Merge into the previous segment: extend end, concatenate text.
                let combined_text = if last.text.trim().is_empty() {
                    seg.text.trim().to_string()
                } else if seg.text.trim().is_empty() {
                    last.text.clone()
                } else {
                    format!("{} {}", last.text.trim(), seg.text.trim())
                };
                last.end_seconds = seg.end_seconds;
                last.text = combined_text;
                // Inherit the longer pause so we don't accidentally clip a gap.
                if seg.pause_after_ms > last.pause_after_ms {
                    last.pause_after_ms = seg.pause_after_ms;
                }
                // Merge frame refs, dedup.
                last.frame_refs.extend(seg.frame_refs.iter());
                last.frame_refs.sort_unstable();
                last.frame_refs.dedup();
                merged_count += 1;
                continue;
            }
        }
        out.push(seg);
    }

    // After merging, tail segment might still be short (no successor to merge
    // into). Fold it back into its predecessor if so.
    if out.len() >= 2 {
        let tail_dur = out
            .last()
            .map(|s| s.end_seconds - s.start_seconds)
            .unwrap_or(0.0);
        if tail_dur < min_duration {
            let tail = out.pop().unwrap();
            let prev = out.last_mut().unwrap();
            let combined = if prev.text.trim().is_empty() {
                tail.text.trim().to_string()
            } else if tail.text.trim().is_empty() {
                prev.text.clone()
            } else {
                format!("{} {}", prev.text.trim(), tail.text.trim())
            };
            prev.end_seconds = tail.end_seconds;
            prev.text = combined;
            if tail.pause_after_ms > prev.pause_after_ms {
                prev.pause_after_ms = tail.pause_after_ms;
            }
            prev.frame_refs.extend(tail.frame_refs.iter());
            prev.frame_refs.sort_unstable();
            prev.frame_refs.dedup();
            merged_count += 1;
        }
    }

    // Re-index after merges.
    for (i, s) in out.iter_mut().enumerate() {
        s.index = i;
    }

    if merged_count > 0 {
        tracing::info!(
            "merge_short_segments: merged {} short (<{:.2}s) segments → {} segments",
            merged_count,
            min_duration,
            out.len()
        );
    }

    out
}

/// Check if an error is a rate limit (429) that should be retried.
fn is_rate_limit_error(err: &NarratorError) -> bool {
    let msg = err.to_string().to_lowercase();
    msg.contains("429")
        || msg.contains("rate limit")
        || msg.contains("too many requests")
        || msg.contains("rate_limit")
        || msg.contains("overloaded")
}

/// Call an AI provider with exponential backoff on rate limit errors.
async fn generate_with_retry(
    provider: &dyn AiProvider,
    system_prompt: &str,
    user_message: serde_json::Value,
) -> Result<String, NarratorError> {
    let max_retries = 4;
    let delays = [5, 15, 30, 60]; // seconds — aggressive backoff for rate limits
    let mut result = Err(NarratorError::ApiError("No attempts made".into()));
    for attempt in 0..=max_retries {
        if attempt > 0 {
            let delay_secs = delays.get(attempt as usize - 1).copied().unwrap_or(60);
            tracing::warn!(
                "Rate limited by API provider. Waiting {delay_secs}s before retry (attempt {attempt}/{max_retries})"
            );
            tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;
        }
        match provider.generate(system_prompt, user_message.clone()).await {
            Ok(text) => return Ok(text),
            Err(e) if is_rate_limit_error(&e) && attempt < max_retries => {
                tracing::warn!("Rate limit error: {e}");
                result = Err(e);
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    result
}

/// Generate narration, chunking the request if there are too many frames.
/// Each chunk gets up to MAX_FRAMES_PER_CALL frames. Subsequent chunks receive
/// context about previously generated segments so the narrative is coherent.
const MAX_FRAMES_PER_CALL: usize = 10;

pub async fn generate_narration(
    provider: &dyn AiProvider,
    system_prompt: &str,
    user_message: serde_json::Value,
    _style: &str,
    _language: &str,
) -> Result<NarrationScript, NarratorError> {
    // Check if the message has too many image parts — if so, chunk it
    let parts = user_message.as_array();
    let image_count = parts
        .map(|p| p.iter().filter(|v| v["type"] == "image").count())
        .unwrap_or(0);
    let was_chunked = image_count > MAX_FRAMES_PER_CALL;

    let response_text = if was_chunked {
        generate_chunked(provider, system_prompt, &user_message, image_count).await?
    } else {
        generate_with_retry(provider, system_prompt, user_message).await?
    };

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

    // Normalize the timeline: filter malformed, sort, dedupe, resolve overlaps.
    // This guarantees monotonic, non-overlapping segments regardless of AI output shape.
    // (For the chunked path this is also applied inside generate_chunked, so running it
    // again here is idempotent and cheap.)
    let duration = if script.total_duration_seconds > 0.0 {
        script.total_duration_seconds
    } else {
        script
            .segments
            .last()
            .map(|s| s.end_seconds)
            .unwrap_or(0.0)
            + 60.0
    };
    script.segments = normalize_timeline(std::mem::take(&mut script.segments), duration);

    // Merge sub-2.5s fragments into their neighbors. Humans can't comfortably
    // speak more than a few words in under 2.5 seconds, and TTS either
    // speeds up unnaturally or overruns when a slot is this short. Merging
    // upfront avoids the audio/video desync downstream.
    script.segments = merge_short_segments(std::mem::take(&mut script.segments), 2.5);

    // Chunked generation is prone to producing a choppy, fragmented script
    // because each chunk only sees a 10-frame window. A single polish pass
    // gives the AI the whole script at once to dedupe, merge, and smooth.
    // Best-effort: if the polish call fails or returns unparseable output we
    // keep the unpolished script rather than breaking generation entirely.
    if was_chunked && script.segments.len() > 3 {
        match polish_script(provider, &script, 2.5).await {
            Ok(polished) => {
                tracing::info!(
                    "AI polish: {} → {} segments",
                    script.segments.len(),
                    polished.segments.len()
                );
                // Re-run normalize + short-merge on the polished output so we
                // guarantee monotonic ordering even if the AI slipped up.
                let mut polished = polished;
                polished.segments =
                    normalize_timeline(std::mem::take(&mut polished.segments), duration);
                polished.segments =
                    merge_short_segments(std::mem::take(&mut polished.segments), 2.5);
                script = polished;
            }
            Err(e) => {
                tracing::warn!("AI polish pass failed, keeping unpolished script: {e}");
            }
        }
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

/// AI polish pass: send the full merged script back to the AI for a
/// holistic review. The model is instructed to:
///   - Remove duplicate or near-duplicate segments
///   - Merge fragmented segments into complete, natural-sounding sentences
///   - Smooth narrative flow (transitions, repetition, word choice)
///   - Flag anomalies (timestamps, contradictions) via re-ordering
///   - Enforce a minimum viable segment duration (caller-specified)
///
/// Returns the polished script on success. Preserves the original metadata
/// (language, provider, generated_at) — those don't need to change.
///
/// Safety: if the AI returns garbage (non-JSON or structurally invalid),
/// the caller is expected to fall back to the unpolished script rather
/// than failing the whole generation.
pub async fn polish_script(
    provider: &dyn AiProvider,
    script: &NarrationScript,
    min_segment_duration: f64,
) -> Result<NarrationScript, NarratorError> {
    let system_prompt = format!(
        "You are a narration script editor performing a holistic polish pass \
         over a complete timed narration. The script was assembled from \
         multiple AI-generated chunks and may contain:\n\
         - Duplicate or near-duplicate segments describing the same thing\n\
         - Fragmented segments that should be one sentence\n\
         - Awkward transitions or repetitive phrasing\n\
         - Segments that are too short for natural speech\n\
         \n\
         Your task:\n\
         1. Keep the overall narrative intact and faithful to the visual content.\n\
         2. Merge adjacent fragmented segments into complete, natural sentences.\n\
         3. Remove duplicate/redundant segments; extend the surviving segment's \
         end_seconds to cover the removed slot.\n\
         4. Ensure every segment has duration >= {min_segment_duration:.1} seconds.\n\
         5. Polish word choice and transitions for a smooth listen — but do NOT \
         rewrite content or add information that wasn't there.\n\
         6. Preserve segment timestamps as much as possible. When merging, use \
         the earliest start_seconds and the latest end_seconds of the merged set.\n\
         7. Segments must remain in strictly ascending time order, non-overlapping.\n\
         \n\
         Respond with ONLY a valid JSON object in the exact same schema as the \
         input (top-level: title, total_duration_seconds, segments, metadata). \
         No markdown code fences, no prose, no explanation."
    );

    let script_json = serde_json::to_string(script)
        .map_err(|e| NarratorError::SerializationError(e.to_string()))?;
    let user_message = json!(script_json);

    let response_text = generate_with_retry(provider, &system_prompt, user_message).await?;
    let json_text = response_text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let mut polished: NarrationScript = serde_json::from_str(json_text).map_err(|e| {
        NarratorError::ApiError(format!(
            "Polish pass returned invalid JSON: {e}\nResponse: {}",
            truncate_chars(json_text, 500)
        ))
    })?;

    // Preserve metadata identity. The polish pass shouldn't change these.
    polished.metadata = script.metadata.clone();
    if polished.total_duration_seconds <= 0.0 {
        polished.total_duration_seconds = script.total_duration_seconds;
    }
    if polished.title.is_empty() {
        polished.title = script.title.clone();
    }

    Ok(polished)
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

    let response_text = generate_with_retry(provider, &system_prompt, user_message).await?;

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

    let response = generate_with_retry(provider, system_prompt, user_message).await?;

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

    // ── normalize_timeline ─────────────────────────────────────────────

    fn seg(index: usize, start: f64, end: f64, text: &str) -> Segment {
        Segment {
            index,
            start_seconds: start,
            end_seconds: end,
            text: text.to_string(),
            visual_description: String::new(),
            emphasis: vec![],
            pace: Pace::default(),
            pause_after_ms: 0,
            frame_refs: vec![],
            voice_override: None,
        }
    }

    #[test]
    fn test_normalize_timeline_sorts_out_of_order() {
        // The exact bug the user reported: segment at 3:22 followed by segment at 2:40
        let segs = vec![
            seg(0, 189.0, 202.0, "segment 16"),
            seg(1, 202.0, 222.0, "segment 17"),
            seg(2, 160.0, 170.0, "segment 18 — goes backwards!"),
            seg(3, 170.0, 180.0, "segment 19"),
        ];
        let out = normalize_timeline(segs, 300.0);
        // Must be in strictly ascending order
        for w in out.windows(2) {
            assert!(
                w[0].end_seconds <= w[1].start_seconds + 0.01,
                "out-of-order: {:?} → {:?}",
                w[0].start_seconds,
                w[1].start_seconds
            );
        }
        // Indexes should be sequential
        for (i, s) in out.iter().enumerate() {
            assert_eq!(s.index, i);
        }
    }

    #[test]
    fn test_normalize_timeline_drops_duplicates() {
        let segs = vec![
            seg(0, 10.0, 20.0, "first"),
            seg(1, 10.05, 20.05, "duplicate"),
            seg(2, 30.0, 40.0, "third"),
        ];
        let out = normalize_timeline(segs, 100.0);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn test_normalize_timeline_drops_fully_contained() {
        let segs = vec![
            seg(0, 10.0, 30.0, "outer"),
            seg(1, 15.0, 25.0, "inside outer"),
            seg(2, 30.0, 40.0, "after"),
        ];
        let out = normalize_timeline(segs, 100.0);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].text, "outer");
        assert_eq!(out[1].text, "after");
    }

    #[test]
    fn test_normalize_timeline_handles_heavy_overlap() {
        // Second segment overlaps by more than 50% → previous is truncated
        let segs = vec![
            seg(0, 10.0, 30.0, "first"),
            seg(1, 12.0, 35.0, "heavy overlap"),
        ];
        let out = normalize_timeline(segs, 100.0);
        assert_eq!(out.len(), 2);
        // The first should be truncated to 12.0
        assert!((out[0].end_seconds - 12.0).abs() < 0.6); // min duration clamps it to 10+0.5
    }

    #[test]
    fn test_normalize_timeline_handles_light_overlap() {
        // Light overlap → push the new segment's start forward
        let segs = vec![
            seg(0, 10.0, 20.0, "first"),
            seg(1, 19.0, 30.0, "light overlap"),
        ];
        let out = normalize_timeline(segs, 100.0);
        assert_eq!(out.len(), 2);
        assert!(out[1].start_seconds >= out[0].end_seconds - 0.01);
    }

    #[test]
    fn test_normalize_timeline_filters_malformed() {
        let segs = vec![
            seg(0, f64::NAN, 10.0, "nan start"),
            seg(1, -5.0, 10.0, "negative start"),
            seg(2, 10.0, 5.0, "end before start"),
            seg(3, 20.0, 30.0, ""),
            seg(4, 40.0, 50.0, "valid"),
        ];
        let out = normalize_timeline(segs, 100.0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "valid");
    }

    #[test]
    fn test_normalize_timeline_clamps_end_to_duration() {
        let segs = vec![seg(0, 10.0, 200.0, "too long")];
        let out = normalize_timeline(segs, 60.0);
        assert_eq!(out.len(), 1);
        assert!((out[0].end_seconds - 60.0).abs() < 0.01);
    }

    #[test]
    fn test_normalize_timeline_enforces_min_duration() {
        let segs = vec![
            seg(0, 10.0, 10.2, "too short"),
            seg(1, 30.0, 40.0, "fine"),
        ];
        let out = normalize_timeline(segs, 100.0);
        assert!(out[0].end_seconds - out[0].start_seconds >= 0.5);
    }

    #[test]
    fn test_normalize_timeline_empty_input() {
        let out = normalize_timeline(vec![], 100.0);
        assert_eq!(out.len(), 0);
    }

    #[test]
    fn test_normalize_timeline_reindexes() {
        let segs = vec![
            seg(99, 30.0, 40.0, "third"),
            seg(42, 10.0, 20.0, "first"),
            seg(7, 20.0, 30.0, "second"),
        ];
        let out = normalize_timeline(segs, 100.0);
        assert_eq!(out[0].index, 0);
        assert_eq!(out[1].index, 1);
        assert_eq!(out[2].index, 2);
        assert_eq!(out[0].text, "first");
    }

    // ── merge_short_segments ──────────────────────────────────────────

    #[test]
    fn test_merge_short_segments_merges_adjacent_fragment() {
        // The exact bug the user reported: many 0.5s segments. Anything
        // shorter than the min-duration floor should fold into its neighbor.
        let segs = vec![
            seg(0, 0.0, 3.0, "First full sentence."),
            seg(1, 3.0, 3.5, "fragment one."),
            seg(2, 3.5, 4.0, "fragment two."),
            seg(3, 4.0, 8.0, "Second full sentence."),
        ];
        let out = merge_short_segments(segs, 2.5);
        // Fragments (0.5s each) should have been merged into the first full
        // segment, which now covers 0.0-4.0s.
        assert_eq!(out.len(), 2, "got {:?}", out.iter().map(|s| (s.start_seconds, s.end_seconds, s.text.clone())).collect::<Vec<_>>());
        assert_eq!(out[0].start_seconds, 0.0);
        assert_eq!(out[0].end_seconds, 4.0);
        assert!(out[0].text.contains("First full sentence"));
        assert!(out[0].text.contains("fragment one"));
        assert!(out[0].text.contains("fragment two"));
        assert_eq!(out[1].text, "Second full sentence.");
    }

    #[test]
    fn test_merge_short_segments_tail_fragment() {
        // Short segment at the tail has no successor; fold into predecessor.
        let segs = vec![
            seg(0, 0.0, 3.0, "Main."),
            seg(1, 3.0, 3.5, "tail fragment."),
        ];
        let out = merge_short_segments(segs, 2.5);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].end_seconds, 3.5);
        assert!(out[0].text.contains("tail fragment"));
    }

    #[test]
    fn test_merge_short_segments_preserves_reindex() {
        let segs = vec![
            seg(0, 0.0, 0.5, "frag"),
            seg(1, 0.5, 3.5, "longer"),
            seg(2, 3.5, 6.5, "another long"),
        ];
        let out = merge_short_segments(segs, 2.5);
        for (i, s) in out.iter().enumerate() {
            assert_eq!(s.index, i);
        }
    }

    #[test]
    fn test_merge_short_segments_noop_when_all_long_enough() {
        let segs = vec![
            seg(0, 0.0, 3.0, "a"),
            seg(1, 3.0, 6.0, "b"),
            seg(2, 6.0, 10.0, "c"),
        ];
        let out = merge_short_segments(segs.clone(), 2.5);
        assert_eq!(out.len(), segs.len());
    }

    #[test]
    fn test_merge_short_segments_single_segment_untouched() {
        let segs = vec![seg(0, 0.0, 1.0, "too short but alone")];
        let out = merge_short_segments(segs, 2.5);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn test_merge_short_segments_consolidates_frame_refs() {
        let mut a = seg(0, 0.0, 3.0, "a");
        a.frame_refs = vec![1, 2];
        let mut b = seg(1, 3.0, 3.5, "b");
        b.frame_refs = vec![2, 3];
        let out = merge_short_segments(vec![a, b], 2.5);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].frame_refs, vec![1, 2, 3]);
    }

    #[test]
    fn test_merge_short_segments_keeps_longer_pause() {
        let mut a = seg(0, 0.0, 3.0, "a");
        a.pause_after_ms = 100;
        let mut b = seg(1, 3.0, 3.5, "b");
        b.pause_after_ms = 500;
        let out = merge_short_segments(vec![a, b], 2.5);
        assert_eq!(out[0].pause_after_ms, 500);
    }

    // ── clamp_chunk_segments ─────────────────────────────────────────────

    #[test]
    fn test_clamp_chunk_segments_drops_outside_range() {
        let segs = vec![
            seg(0, 5.0, 15.0, "partial start overlap — clamped"),
            seg(1, 20.0, 30.0, "fully inside"),
            seg(2, 100.0, 110.0, "fully after end — clamped to nothing"),
        ];
        let out = clamp_chunk_segments(segs, 10.0, 40.0);
        // The first is clamped to [10, 15], the second fully survives,
        // the third is clamped to [40, 40] which fails min duration.
        assert_eq!(out.len(), 2);
        assert!((out[0].start_seconds - 10.0).abs() < 0.01);
    }

    #[test]
    fn test_clamp_chunk_segments_drops_too_short() {
        let segs = vec![seg(0, 5.0, 10.1, "too short after clamp")];
        let out = clamp_chunk_segments(segs, 10.0, 100.0);
        assert_eq!(out.len(), 0);
    }

    // ── Chunked generation ────────────────────────────────────────────────

    /// Mock provider that returns a different response per call, and records
    /// every system_prompt / user_message pair it received.
    struct ScriptedProvider {
        responses: std::sync::Mutex<Vec<String>>,
        calls: std::sync::Mutex<Vec<serde_json::Value>>,
    }

    impl ScriptedProvider {
        fn new(responses: Vec<&str>) -> Self {
            Self {
                responses: std::sync::Mutex::new(
                    responses.into_iter().map(String::from).collect(),
                ),
                calls: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn captured(&self) -> Vec<serde_json::Value> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl AiProvider for ScriptedProvider {
        async fn generate(
            &self,
            _system_prompt: &str,
            user_message: serde_json::Value,
        ) -> Result<String, NarratorError> {
            self.calls.lock().unwrap().push(user_message);
            let mut r = self.responses.lock().unwrap();
            if r.is_empty() {
                return Err(NarratorError::ApiError(
                    "ScriptedProvider ran out of responses".into(),
                ));
            }
            Ok(r.remove(0))
        }
        fn name(&self) -> &str {
            "scripted"
        }
        fn model(&self) -> &str {
            "scripted-v1"
        }
    }

    fn chunk_response_json(title: &str, total: f64, segs: &[(f64, f64, &str)]) -> String {
        let seg_json: Vec<serde_json::Value> = segs
            .iter()
            .enumerate()
            .map(|(i, (s, e, t))| {
                json!({
                    "index": i,
                    "start_seconds": s,
                    "end_seconds": e,
                    "text": t,
                    "visual_description": "",
                    "emphasis": [],
                    "pace": "medium",
                    "pause_after_ms": 0,
                    "frame_refs": []
                })
            })
            .collect();
        json!({
            "title": title,
            "total_duration_seconds": total,
            "segments": seg_json,
            "metadata": {
                "style": "test",
                "language": "en",
                "provider": "scripted",
                "model": "scripted-v1",
                "generated_at": "2026-04-01T00:00:00Z"
            }
        })
        .to_string()
    }

    /// Build a user_message with exactly `num_frames` image parts, each labeled
    /// with a timestamp. Frame i is labelled "[Frame {i} at {i*interval:.1}s]".
    fn user_msg_with_frames(num_frames: usize, interval: f64) -> serde_json::Value {
        let mut parts: Vec<serde_json::Value> = vec![json!({
            "type": "text",
            "text": "Context: test video."
        })];
        for i in 0..num_frames {
            parts.push(json!({
                "type": "text",
                "text": format!("[Frame {} at {:.1}s]", i, i as f64 * interval)
            }));
            parts.push(json!({
                "type": "image",
                "source": { "type": "base64", "data": "QQ==" }
            }));
        }
        serde_json::Value::Array(parts)
    }

    #[tokio::test]
    async fn test_chunked_generation_splits_frames() {
        // 25 frames at 1s intervals → 3 chunks of 10, 10, 5 (MAX_FRAMES_PER_CALL=10)
        let r1 = chunk_response_json(
            "T",
            25.0,
            &[(0.0, 5.0, "chunk1 first"), (5.0, 10.0, "chunk1 second")],
        );
        let r2 = chunk_response_json(
            "T",
            25.0,
            &[(10.0, 15.0, "chunk2 first"), (15.0, 20.0, "chunk2 second")],
        );
        let r3 = chunk_response_json("T", 25.0, &[(20.0, 25.0, "chunk3")]);
        // With > 3 merged segments the pipeline runs an AI polish pass too,
        // so 4 provider calls total.
        let polish_response = chunk_response_json(
            "T",
            25.0,
            &[
                (0.0, 5.0, "chunk1 first"),
                (5.0, 10.0, "chunk1 second"),
                (10.0, 15.0, "chunk2 first"),
                (15.0, 20.0, "chunk2 second"),
                (20.0, 25.0, "chunk3"),
            ],
        );
        let provider =
            ScriptedProvider::new(vec![&r1, &r2, &r3, &polish_response]);

        let msg = user_msg_with_frames(25, 1.0);
        let result =
            generate_narration(&provider, "sys", msg, "test", "en").await.unwrap();

        // 3 chunk calls + 1 polish call
        assert_eq!(provider.captured().len(), 4);
        // Merged script has 5 segments (2+2+1)
        assert_eq!(result.segments.len(), 5);
        // Segments must be in strictly ascending order
        for w in result.segments.windows(2) {
            assert!(
                w[0].end_seconds <= w[1].start_seconds + 0.01,
                "segments out of order: {} vs {}",
                w[0].end_seconds,
                w[1].start_seconds
            );
        }
        // Indexes should be sequential after normalize_timeline
        for (i, s) in result.segments.iter().enumerate() {
            assert_eq!(s.index, i);
        }
    }

    #[tokio::test]
    async fn test_chunked_generation_fixes_backwards_segments() {
        // Simulates the exact bug the user reported: chunk 2 returns segments
        // with timestamps BEFORE chunk 1's last segment. normalize_timeline
        // must enforce strictly-ascending order.
        let r1 = chunk_response_json(
            "T",
            30.0,
            &[(0.0, 10.0, "chunk1 A"), (10.0, 20.0, "chunk1 B")],
        );
        // Chunk 2 emits a segment at 5-8s — BEFORE chunk 1's end.
        let r2 = chunk_response_json(
            "T",
            30.0,
            &[
                (5.0, 8.0, "backwards jump!"),
                (22.0, 28.0, "later"),
            ],
        );
        let provider = ScriptedProvider::new(vec![&r1, &r2]);
        let msg = user_msg_with_frames(15, 2.0); // 2 chunks

        let result =
            generate_narration(&provider, "sys", msg, "test", "en").await.unwrap();

        // The "backwards jump" segment must NOT appear before chunk1's segments
        for w in result.segments.windows(2) {
            assert!(
                w[0].end_seconds <= w[1].start_seconds + 0.01,
                "backwards jump slipped through: {} -> {}",
                w[0].end_seconds,
                w[1].start_seconds
            );
        }
    }

    #[tokio::test]
    async fn test_chunked_generation_time_bounds_in_prompt() {
        // Each chunk prompt should include STRICT TIME BOUNDS instruction.
        let r1 = chunk_response_json("T", 30.0, &[(0.0, 10.0, "a")]);
        let r2 = chunk_response_json("T", 30.0, &[(15.0, 25.0, "b")]);
        let provider = ScriptedProvider::new(vec![&r1, &r2]);
        let msg = user_msg_with_frames(15, 2.0);

        generate_narration(&provider, "sys", msg, "test", "en").await.unwrap();

        let calls = provider.captured();
        assert_eq!(calls.len(), 2);
        for (i, call) in calls.iter().enumerate() {
            let arr = call.as_array().expect("user message should be array");
            let text_parts: Vec<String> = arr
                .iter()
                .filter_map(|p| p.get("text").and_then(|v| v.as_str()).map(String::from))
                .collect();
            let combined = text_parts.join("\n");
            assert!(
                combined.contains("STRICT TIME BOUNDS"),
                "chunk {} prompt missing time-bounds instruction:\n{}",
                i + 1,
                combined
            );
            assert!(
                combined.contains("start_seconds >="),
                "chunk {} prompt missing start_seconds constraint",
                i + 1
            );
        }
    }

    #[tokio::test]
    async fn test_chunked_generation_drops_segments_outside_chunk_bounds() {
        // Chunk 1 covers frames 0..10 (times 0–9s). AI hallucinates a segment
        // at 50s which should be clamped away before merge.
        let r1 = chunk_response_json(
            "T",
            30.0,
            &[
                (0.0, 5.0, "valid"),
                (50.0, 60.0, "wildly out of range"),
            ],
        );
        let r2 = chunk_response_json("T", 30.0, &[(15.0, 20.0, "chunk2")]);
        let provider = ScriptedProvider::new(vec![&r1, &r2]);
        let msg = user_msg_with_frames(15, 2.0);

        let result =
            generate_narration(&provider, "sys", msg, "test", "en").await.unwrap();

        // The out-of-range segment shouldn't survive
        assert!(
            !result
                .segments
                .iter()
                .any(|s| s.text == "wildly out of range"),
            "out-of-bounds segment leaked through"
        );
    }

    #[tokio::test]
    async fn test_single_call_generation_applies_normalize() {
        // Single call (<= MAX_FRAMES_PER_CALL frames) should still apply
        // normalize_timeline to fix any out-of-order segments the AI emits.
        let resp = chunk_response_json(
            "T",
            30.0,
            &[
                (10.0, 20.0, "second"),
                (0.0, 10.0, "first"),
                (20.0, 30.0, "third"),
            ],
        );
        let provider = ScriptedProvider::new(vec![&resp]);
        let msg = user_msg_with_frames(3, 5.0);

        let result =
            generate_narration(&provider, "sys", msg, "test", "en").await.unwrap();

        assert_eq!(provider.captured().len(), 1);
        assert_eq!(result.segments.len(), 3);
        // Must be sorted
        assert!(result.segments[0].start_seconds < result.segments[1].start_seconds);
        assert!(result.segments[1].start_seconds < result.segments[2].start_seconds);
        assert_eq!(result.segments[0].text, "first");
    }

    #[tokio::test]
    async fn test_chunked_generation_prev_context_included() {
        // Chunks after the first should include "PREVIOUSLY GENERATED SEGMENTS"
        // so the AI can continue coherently.
        let r1 = chunk_response_json("T", 30.0, &[(0.0, 5.0, "hello world")]);
        let r2 = chunk_response_json("T", 30.0, &[(15.0, 20.0, "continued")]);
        let provider = ScriptedProvider::new(vec![&r1, &r2]);
        let msg = user_msg_with_frames(15, 2.0);

        generate_narration(&provider, "sys", msg, "test", "en").await.unwrap();

        let calls = provider.captured();
        assert_eq!(calls.len(), 2);
        let second_call_text: String = calls[1]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|p| p.get("text").and_then(|v| v.as_str()).map(String::from))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            second_call_text.contains("PREVIOUSLY GENERATED SEGMENTS"),
            "second chunk missing context from first"
        );
        assert!(
            second_call_text.contains("hello world"),
            "second chunk missing previous segment text"
        );
    }

    // ── truncate_chars: UTF-8 safety ────────────────────────────────────

    #[test]
    fn test_truncate_chars_ascii() {
        assert_eq!(truncate_chars("hello world", 5), "hello");
        assert_eq!(truncate_chars("short", 100), "short");
    }

    #[test]
    fn test_truncate_chars_preserves_multibyte_boundaries() {
        // Japanese text — would panic with naive byte slicing
        let japanese = "こんにちは世界"; // 7 chars, 21 bytes in UTF-8
        let result = truncate_chars(japanese, 3);
        assert_eq!(result, "こんに");
        assert_eq!(result.chars().count(), 3);
    }

    #[test]
    fn test_truncate_chars_emoji() {
        // Emoji are 4-byte sequences — same panic risk
        let text = "Done 🎬 now 🎞️ what";
        let result = truncate_chars(text, 7);
        // First 7 chars (whatever they are) — key assertion is no panic
        assert!(result.chars().count() <= 7);
    }

    #[test]
    fn test_truncate_chars_boundary_cases() {
        assert_eq!(truncate_chars("", 10), "");
        assert_eq!(truncate_chars("a", 0), "");
        assert_eq!(truncate_chars("abc", 3), "abc");
    }

    // ── Chunked generation: more edge cases ─────────────────────────────

    #[tokio::test]
    async fn test_chunked_generation_with_multibyte_segments() {
        // Segments with Japanese text should not cause panics in logging/truncation.
        // The trigger: a segment with length >60 bytes where byte-index 60 falls
        // mid-codepoint. In the chunk-overlap warn branch, we log truncate_chars(text, 60).
        let long_jp = "こんにちは世界これはナレーションのテキストです".repeat(2);
        let r1 = chunk_response_json(
            "T",
            30.0,
            &[(0.0, 10.0, &long_jp), (10.0, 20.0, &long_jp)],
        );
        let r2 = chunk_response_json(
            "T",
            30.0,
            &[
                // Deliberately fully-contained in chunk 1's range → triggers the
                // "Dropping segment fully contained" warn log that uses
                // truncate_chars(&seg.text, 60) under the hood.
                (5.0, 8.0, &long_jp),
                (22.0, 28.0, &long_jp),
            ],
        );
        let provider = ScriptedProvider::new(vec![&r1, &r2]);
        let msg = user_msg_with_frames(15, 2.0);

        let result = generate_narration(&provider, "sys", msg, "test", "en")
            .await
            .expect("must not panic on multibyte text");
        // Monotonic order preserved
        for w in result.segments.windows(2) {
            assert!(w[0].end_seconds <= w[1].start_seconds + 0.01);
        }
    }

    #[tokio::test]
    async fn test_chunked_generation_empty_segments_response() {
        // Chunk 1 returns no segments; chunk 2 returns a segment within ITS bounds.
        // With 15 frames at 2s interval, chunks are [0..10] (times 0-18) and
        // [10..15] (times 20-28). Chunk 2 bounds are [20, 58] — so we place
        // the segment at 22-26s.
        let r1 = chunk_response_json("T", 30.0, &[]);
        let r2 = chunk_response_json("T", 30.0, &[(22.0, 26.0, "only segment")]);
        let provider = ScriptedProvider::new(vec![&r1, &r2]);
        let msg = user_msg_with_frames(15, 2.0);

        let result = generate_narration(&provider, "sys", msg, "test", "en")
            .await
            .unwrap();
        assert_eq!(result.segments.len(), 1);
        assert_eq!(result.segments[0].text, "only segment");
    }

    #[tokio::test]
    async fn test_single_call_unicode_survives_normalize() {
        let resp = chunk_response_json(
            "Tタイトル",
            30.0,
            &[
                (0.0, 10.0, "セグメント 1 🎬"),
                (10.0, 20.0, "セグメント 2 🎞️"),
                (20.0, 30.0, "セグメント 3 ✂️"),
            ],
        );
        let provider = ScriptedProvider::new(vec![&resp]);
        let msg = user_msg_with_frames(3, 5.0);

        let result = generate_narration(&provider, "sys", msg, "test", "ja")
            .await
            .unwrap();
        assert_eq!(result.segments.len(), 3);
        assert_eq!(result.segments[0].text, "セグメント 1 🎬");
    }

    #[tokio::test]
    async fn test_single_call_at_max_frames_threshold() {
        // Exactly MAX_FRAMES_PER_CALL frames → single-call path, not chunked
        let resp =
            chunk_response_json("T", 30.0, &[(0.0, 15.0, "a"), (15.0, 30.0, "b")]);
        let provider = ScriptedProvider::new(vec![&resp]);
        let msg = user_msg_with_frames(MAX_FRAMES_PER_CALL, 1.0);

        generate_narration(&provider, "sys", msg, "test", "en")
            .await
            .unwrap();
        // Exactly one call — confirms the threshold check is `>` not `>=`
        assert_eq!(provider.captured().len(), 1);
    }

    #[tokio::test]
    async fn test_chunked_generation_just_over_threshold() {
        // MAX_FRAMES_PER_CALL + 1 → 2 chunks of 10 and 1
        let r1 = chunk_response_json("T", 30.0, &[(0.0, 15.0, "chunk1")]);
        let r2 = chunk_response_json("T", 30.0, &[(20.0, 25.0, "chunk2")]);
        let provider = ScriptedProvider::new(vec![&r1, &r2]);
        let msg = user_msg_with_frames(MAX_FRAMES_PER_CALL + 1, 1.0);

        generate_narration(&provider, "sys", msg, "test", "en")
            .await
            .unwrap();
        assert_eq!(provider.captured().len(), 2);
    }

    #[tokio::test]
    async fn test_chunked_generation_frames_without_timestamps() {
        // Frames whose labels don't match "[Frame N at X.Xs]" → timestamps parse
        // as 0.0 and the pipeline should still function (even if chunk bounds
        // are degenerate).
        let r1 = chunk_response_json("T", 30.0, &[(0.0, 5.0, "a")]);
        let r2 = chunk_response_json("T", 30.0, &[(10.0, 15.0, "b")]);
        let provider = ScriptedProvider::new(vec![&r1, &r2]);

        // Build a message with unlabeled image pairs
        let mut parts: Vec<serde_json::Value> = vec![json!({"type":"text","text":"ctx"})];
        for _ in 0..15 {
            parts.push(json!({"type":"text","text":"frame"}));
            parts.push(json!({"type":"image","source":{"type":"base64","data":"QQ=="}}));
        }
        let msg = serde_json::Value::Array(parts);

        let result = generate_narration(&provider, "sys", msg, "test", "en").await;
        assert!(result.is_ok(), "should still succeed with unparseable timestamps");
    }

    #[tokio::test]
    async fn test_chunked_generation_invalid_json_chunk_errors_cleanly() {
        let r1 = chunk_response_json("T", 30.0, &[(0.0, 5.0, "ok")]);
        // Chunk 2 returns garbage — should bubble up a clear ApiError
        let provider = ScriptedProvider::new(vec![&r1, "NOT JSON AT ALL"]);
        let msg = user_msg_with_frames(15, 2.0);

        let err = generate_narration(&provider, "sys", msg, "test", "en")
            .await
            .unwrap_err();
        let err_str = err.to_string();
        assert!(
            err_str.contains("chunk 2") || err_str.contains("parse"),
            "expected parse error for chunk 2, got: {err_str}"
        );
    }

    #[tokio::test]
    async fn test_chunked_generation_first_chunk_must_start_at_zero() {
        // Verify the first chunk prompt instructs start from 0.0
        let r1 = chunk_response_json("T", 30.0, &[(0.0, 5.0, "a")]);
        let r2 = chunk_response_json("T", 30.0, &[(20.0, 25.0, "b")]);
        let provider = ScriptedProvider::new(vec![&r1, &r2]);
        let msg = user_msg_with_frames(15, 2.0);

        generate_narration(&provider, "sys", msg, "test", "en").await.unwrap();
        let first_call_text: String = provider.captured()[0]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|p| p.get("text").and_then(|v| v.as_str()).map(String::from))
            .collect::<Vec<_>>()
            .join("\n");
        // First chunk's lower bound should be 0.00
        assert!(
            first_call_text.contains("start_seconds >= 0.00"),
            "first chunk missing start>=0 instruction"
        );
    }

    #[tokio::test]
    async fn test_chunked_handles_all_chunks_empty() {
        // Pathological case: every chunk returns no segments
        let r1 = chunk_response_json("T", 30.0, &[]);
        let r2 = chunk_response_json("T", 30.0, &[]);
        let provider = ScriptedProvider::new(vec![&r1, &r2]);
        let msg = user_msg_with_frames(15, 2.0);

        let result = generate_narration(&provider, "sys", msg, "test", "en")
            .await
            .unwrap();
        // Should succeed with empty segments — not crash
        assert_eq!(result.segments.len(), 0);
    }

    // ── translate_script ───────────────────────────────────────────────

    fn translation_response_json(title: &str, lang: &str, segs: &[(f64, f64, &str)]) -> String {
        chunk_response_json(title, 30.0, segs)
            // Override language in metadata (simulate the AI returning the translated script)
            .replacen("\"language\":\"en\"", &format!("\"language\":\"{lang}\""), 1)
    }

    #[tokio::test]
    async fn test_translate_script_success() {
        let original = NarrationScript {
            title: "Original".into(),
            total_duration_seconds: 30.0,
            segments: vec![
                Segment {
                    index: 0,
                    start_seconds: 0.0,
                    end_seconds: 15.0,
                    text: "Hello world".into(),
                    visual_description: String::new(),
                    emphasis: vec![],
                    pace: Pace::Medium,
                    pause_after_ms: 0,
                    frame_refs: vec![],
                    voice_override: None,
                },
            ],
            metadata: ScriptMetadata {
                style: "test".into(),
                language: "en".into(),
                provider: "mock".into(),
                model: "mock-v1".into(),
                generated_at: "2026-04-01T00:00:00Z".into(),
            },
        };
        let resp = translation_response_json(
            "Original",
            "ja",
            &[(0.0, 15.0, "こんにちは世界")],
        );
        let provider = ScriptedProvider::new(vec![&resp]);

        let translated = translate_script(&provider, &original, "Japanese")
            .await
            .unwrap();
        // Metadata language should be set to target
        assert_eq!(translated.metadata.language, "Japanese");
        assert_eq!(translated.segments.len(), 1);
        assert_eq!(translated.segments[0].text, "こんにちは世界");
    }

    #[tokio::test]
    async fn test_translate_script_strips_code_fences() {
        let original = NarrationScript {
            title: "T".into(),
            total_duration_seconds: 10.0,
            segments: vec![],
            metadata: ScriptMetadata {
                style: "test".into(),
                language: "en".into(),
                provider: "mock".into(),
                model: "mock-v1".into(),
                generated_at: "2026-01-01T00:00:00Z".into(),
            },
        };
        let fenced = format!(
            "```json\n{}\n```",
            chunk_response_json("T", 10.0, &[(0.0, 5.0, "hola")])
        );
        let provider = ScriptedProvider::new(vec![&fenced]);
        let result = translate_script(&provider, &original, "es").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_translate_script_invalid_json_errors() {
        let original = NarrationScript {
            title: "T".into(),
            total_duration_seconds: 10.0,
            segments: vec![],
            metadata: ScriptMetadata {
                style: "test".into(),
                language: "en".into(),
                provider: "mock".into(),
                model: "mock-v1".into(),
                generated_at: "2026-01-01T00:00:00Z".into(),
            },
        };
        let provider = ScriptedProvider::new(vec!["not json at all"]);
        let err = translate_script(&provider, &original, "fr")
            .await
            .unwrap_err();
        assert!(err.to_string().to_lowercase().contains("parse"));
    }

    // ── refine_segment ─────────────────────────────────────────────────

    #[tokio::test]
    async fn test_refine_segment_returns_clean_text() {
        let provider = ScriptedProvider::new(vec!["This is the refined text."]);
        let result =
            refine_segment(&provider, "original text", "make shorter", "surrounding context")
                .await
                .unwrap();
        assert_eq!(result, "This is the refined text.");
    }

    #[tokio::test]
    async fn test_refine_segment_strips_quotes_and_fences() {
        let provider =
            ScriptedProvider::new(vec!["```\n\"Quoted refinement\"\n```"]);
        let result = refine_segment(&provider, "orig", "instruction", "ctx")
            .await
            .unwrap();
        // Leading/trailing quotes and code fences removed
        assert!(!result.contains("```"));
        assert!(result.contains("Quoted refinement"));
    }

    #[tokio::test]
    async fn test_refine_segment_empty_response_errors() {
        let provider = ScriptedProvider::new(vec!["   \n\n   "]);
        let err = refine_segment(&provider, "orig", "inst", "ctx")
            .await
            .unwrap_err();
        assert!(err.to_string().to_lowercase().contains("empty"));
    }

    #[tokio::test]
    async fn test_refine_segment_preserves_unicode() {
        let provider =
            ScriptedProvider::new(vec!["精緻化されたセグメント 🎬"]);
        let result = refine_segment(&provider, "orig", "inst", "ctx")
            .await
            .unwrap();
        assert_eq!(result, "精緻化されたセグメント 🎬");
    }

    // ── polish_script ────────────────────────────────────────────────

    fn sample_script() -> NarrationScript {
        NarrationScript {
            title: "Test".into(),
            total_duration_seconds: 30.0,
            segments: vec![
                seg(0, 0.0, 3.0, "first"),
                seg(1, 3.0, 3.5, "frag"),
                seg(2, 3.5, 10.0, "second"),
            ],
            metadata: ScriptMetadata {
                style: "test".into(),
                language: "en".into(),
                provider: "mock".into(),
                model: "mock-v1".into(),
                generated_at: "2026-01-01T00:00:00Z".into(),
            },
        }
    }

    #[tokio::test]
    async fn test_polish_script_applies_ai_changes() {
        // AI merges the fragment into the first segment
        let resp = chunk_response_json(
            "Test",
            30.0,
            &[(0.0, 3.5, "first frag"), (3.5, 10.0, "second")],
        );
        let provider = ScriptedProvider::new(vec![&resp]);
        let result = polish_script(&provider, &sample_script(), 2.5)
            .await
            .unwrap();
        assert_eq!(result.segments.len(), 2);
        assert!(result.segments[0].text.contains("first frag"));
    }

    #[tokio::test]
    async fn test_polish_script_preserves_metadata() {
        let resp = chunk_response_json("Test", 30.0, &[(0.0, 10.0, "one")]);
        let provider = ScriptedProvider::new(vec![&resp]);
        let original = sample_script();
        let result = polish_script(&provider, &original, 2.5).await.unwrap();
        // Metadata identity preserved from input, not from AI response
        assert_eq!(result.metadata.language, original.metadata.language);
        assert_eq!(result.metadata.provider, original.metadata.provider);
        assert_eq!(result.metadata.generated_at, original.metadata.generated_at);
    }

    #[tokio::test]
    async fn test_polish_script_invalid_json_errors() {
        let provider = ScriptedProvider::new(vec!["definitely not json"]);
        let err = polish_script(&provider, &sample_script(), 2.5)
            .await
            .unwrap_err();
        assert!(err.to_string().to_lowercase().contains("polish"));
    }

    #[tokio::test]
    async fn test_polish_script_strips_code_fences() {
        let inner = chunk_response_json("Test", 30.0, &[(0.0, 10.0, "one")]);
        let fenced = format!("```json\n{inner}\n```");
        let provider = ScriptedProvider::new(vec![&fenced]);
        let result = polish_script(&provider, &sample_script(), 2.5).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_polish_script_falls_back_title_and_duration() {
        // AI returned empty title and zero total_duration → fall back to input
        let resp = r#"{
            "title": "",
            "total_duration_seconds": 0,
            "segments": [{"index":0,"start_seconds":0,"end_seconds":10,"text":"ok","visual_description":"","emphasis":[],"pace":"medium","pause_after_ms":0,"frame_refs":[]}],
            "metadata": {"style":"","language":"","provider":"","model":"","generated_at":""}
        }"#;
        let provider = ScriptedProvider::new(vec![resp]);
        let result = polish_script(&provider, &sample_script(), 2.5).await.unwrap();
        assert_eq!(result.title, "Test");
        assert_eq!(result.total_duration_seconds, 30.0);
    }
}
