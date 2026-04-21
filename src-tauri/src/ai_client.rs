//! Multi-provider AI client supporting Claude and OpenAI for narration generation.

use crate::error::NarratorError;
use crate::http_client;
use crate::models::*;
use crate::video_engine;
use async_trait::async_trait;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Callback invoked as each new segment is produced. Used by the command layer
/// to push `ProgressEvent::SegmentStreamed` over the progress channel so the UI
/// can render partial progress — and, on failure, so the frontend can pass the
/// same segments back via `resume_segments` to skip completed chunks on retry.
pub type SegmentCallback = Arc<dyn Fn(&Segment) + Send + Sync>;

/// Callback invoked with a coarse (fraction, message) pair at chunk boundaries.
/// `fraction` is 0..=1 of *the narration stage*, not the global progress bar —
/// the caller re-scales. `message` is the label users see under the progress
/// bar ("Analyzing batch 2 of 5"). `None` means "keep the previous label".
pub type ProgressCallback = Arc<dyn Fn(f64, Option<String>) + Send + Sync>;

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
                tracing::error!("API error ({status}): {}", truncate_chars(&error_text, 400));
                let api_msg = parse_api_error_message(&error_text);
                let hint = match status.as_u16() {
                    401 | 403 => "Check that your Anthropic API key is valid.",
                    400 => "The request was rejected — usually a model or parameter mismatch.",
                    _ => "See the details below.",
                };
                let detail: String = if api_msg.is_empty() {
                    truncate_chars(&error_text, 240).into_owned()
                } else {
                    api_msg
                };
                return Err(NarratorError::ApiError(format!(
                    "Claude API error (HTTP {status}). {hint}\n\n{detail}"
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

/// Reasoning-model families (OpenAI) that do not accept user-set
/// `temperature` and require `max_completion_tokens` in place of
/// `max_tokens`: o1/o3/o4 and the GPT-5 family. Sending `temperature`
/// to these models produces a 400 with `invalid_request_error`.
pub fn is_openai_reasoning_model(model: &str) -> bool {
    model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
        || model.starts_with("gpt-5")
}

/// Best-effort extraction of the `error.message` field from a JSON error
/// body. All three providers (OpenAI / Anthropic / Gemini) put the
/// human-readable explanation there. Returns an empty string when the
/// body isn't JSON or the field is missing — callers should fall back
/// to the raw body in that case.
fn parse_api_error_message(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(|s| s.to_string()))
        .unwrap_or_default()
}

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

        // Reasoning models (o1/o3/o4, gpt-5) require `max_completion_tokens`
        // instead of `max_tokens` AND reject any user-set `temperature`
        // (only the default of 1 is accepted — sending even 1.0 explicitly
        // has been observed to fail on some checkpoints, so we omit it).
        let is_reasoning_model = is_openai_reasoning_model(&self.model);
        let token_key = if is_reasoning_model {
            "max_completion_tokens"
        } else {
            "max_tokens"
        };
        let mut body = json!({
            "model": self.model,
            token_key: 8192,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_content}
            ]
        });
        if !is_reasoning_model {
            body["temperature"] = json!(self.temperature);
        }

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
                tracing::error!("API error ({status}): {}", truncate_chars(&error_text, 400));
                let api_msg = parse_api_error_message(&error_text);
                // 401/403 really is auth; 400 etc. is usually a malformed
                // request (model, temperature, etc.) — don't say "check
                // your key" when it's actually a shape problem.
                let hint = match status.as_u16() {
                    401 | 403 => "Check that your OpenAI API key is valid.",
                    400 => "The request was rejected — usually a model or parameter mismatch.",
                    _ => "See the details below.",
                };
                let detail: String = if api_msg.is_empty() {
                    truncate_chars(&error_text, 240).into_owned()
                } else {
                    api_msg
                };
                return Err(NarratorError::ApiError(format!(
                    "OpenAI API error (HTTP {status}). {hint}\n\n{detail}"
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
                tracing::error!("API error ({status}): {}", truncate_chars(&error_text, 400));
                let api_msg = parse_api_error_message(&error_text);
                let hint = match status.as_u16() {
                    401 | 403 => "Check that your Google API key is valid.",
                    400 => "The request was rejected — usually a model or parameter mismatch.",
                    _ => "See the details below.",
                };
                let detail: String = if api_msg.is_empty() {
                    truncate_chars(&error_text, 240).into_owned()
                } else {
                    api_msg
                };
                return Err(NarratorError::ApiError(format!(
                    "Gemini API error (HTTP {status}). {hint}\n\n{detail}"
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
    lang: &str,
) -> String {
    let mut prompt = String::new();

    let target_rate = crate::speech_rate::rate_per_minute(lang);
    let unit = crate::speech_rate::budget_unit(lang);

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
        8. Prefer substantive narration when the per-segment word budget allows \
           (see WORD BUDGET below). When the budget is tight, keep the segment \
           short — do NOT cram extra detail into a small time window.\n\n",
    );

    // Word budget — the single most important constraint. Without this the LLM
    // happily emits 550 wpm text that TTS cannot possibly deliver in the window.
    prompt.push_str(&format!(
        "## WORD BUDGET (hard constraint)\n\n\
        The text-to-speech engine for language '{lang}' delivers roughly \
        {target_rate:.0} {unit} per minute at natural pace. For EVERY segment, \
        the `text` field MUST fit inside `end_seconds - start_seconds` at that \
        rate. That means:\n\n\
        \tmax_{unit} = round((end_seconds - start_seconds) × {target_rate:.0} / 60)\n\n\
        If an idea doesn't fit the budget, do ONE of these — never cram text:\n\
        \t• Extend `end_seconds` (borrow from the gap before the next segment).\n\
        \t• Split the idea into two adjacent segments.\n\
        \t• Cut the idea down — trim adjectives, drop asides.\n\n\
        A segment whose speech duration exceeds its window causes the exported \
        video to visibly desync or stretch. Treat the budget as a hard upper \
        bound, not a suggestion.\n\n"
    ));

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

/// Wrap an async API call with a periodic progress heartbeat so the UI doesn't
/// look frozen during a 10-30s Claude call. Emits a progress event every
/// ~1.5s with `label · Ns elapsed` at the same `fraction` the caller would
/// have emitted on its own — we only change the message, never creep the
/// percent, so this can't interact badly with the frontend's monotonic clamp.
///
/// The inner future is awaited; the heartbeat is cancelled as soon as it
/// resolves (success or error). If the heartbeat is None (no progress
/// callback), the inner future runs as-is.
async fn with_heartbeat<F, T>(
    on_progress: &Option<ProgressCallback>,
    fraction: f64,
    label: String,
    fut: F,
) -> T
where
    F: std::future::Future<Output = T>,
{
    let Some(cb) = on_progress.clone() else {
        return fut.await;
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel.clone();
    let label_clone = label.clone();
    let started = std::time::Instant::now();
    let handle = tokio::spawn(async move {
        // Short initial delay: if the inner future finishes in <1s we skip the
        // first tick entirely and avoid flicker.
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        while !cancel_clone.load(Ordering::SeqCst) {
            let elapsed = started.elapsed().as_secs();
            cb(
                fraction,
                Some(format!("{label_clone} · {elapsed}s elapsed")),
            );
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        }
    });
    let result = fut.await;
    cancel.store(true, Ordering::SeqCst);
    // Abort the tick task so it doesn't survive past the function. `abort` is
    // race-free — if the task already exited via the cancel check, this is a
    // no-op; otherwise it cancels between ticks so we never double-report.
    handle.abort();
    result
}

/// Generate narration in chunks when there are too many frames for a single API call.
/// Splits frames into batches, generates segments per batch with context from previous.
///
/// `resume_segments`: segments already produced by a prior partial run. The loop
/// seeds its accumulator with these and skips any chunk whose frames are fully
/// before the last resumed segment's `end_seconds`, so the API isn't billed
/// again for work that already succeeded.
///
/// `on_segment`: called once for each newly-produced, kept segment (after
/// clamping + ordering checks). Not called for resumed segments — the caller
/// already has them. Use this to stream partial progress to the UI so users
/// can see what's been generated mid-flight.
#[allow(clippy::too_many_arguments)]
async fn generate_chunked(
    provider: &dyn AiProvider,
    system_prompt: &str,
    user_message: &serde_json::Value,
    image_count: usize,
    resume_segments: Vec<Segment>,
    on_segment: Option<SegmentCallback>,
    on_progress: Option<ProgressCallback>,
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

    // Seed the accumulator with any resumed segments so subsequent chunks
    // get coherent `prev_summary` context and continue from the right
    // timestamp. `resume_cutoff` is used below to skip chunks the prior run
    // already completed.
    let resume_cutoff = resume_segments.last().map(|s| s.end_seconds).unwrap_or(0.0);
    let had_resume_segments = !resume_segments.is_empty();
    let mut all_segments: Vec<crate::models::Segment> = resume_segments;
    let mut merged_script: Option<NarrationScript> = None;
    let mut skipped_chunks = 0usize;
    let mut emitted_resume_jump = !had_resume_segments;

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

        // Skip chunks already covered by resume_segments. A chunk is covered
        // when its last frame timestamp is at or before the resumed cutoff —
        // meaning every frame in this chunk was part of the prior successful
        // run and regenerating would waste an API call.
        if resume_cutoff > 0.0 && chunk_last_ts <= resume_cutoff + 0.01 {
            skipped_chunks += 1;
            tracing::info!(
                "Chunk {}/{} skipped (frames {:.2}s–{:.2}s covered by resume cutoff {:.2}s)",
                chunk_idx + 1,
                num_chunks,
                chunk_first_ts,
                chunk_last_ts,
                resume_cutoff
            );
            continue;
        }

        // First live chunk after a resume: jump the progress bar forward to
        // reflect completed work so the user doesn't watch the bar rebuild
        // from 0%. Emitted once, only when we actually start running.
        if !emitted_resume_jump {
            emitted_resume_jump = true;
            if let Some(cb) = on_progress.as_ref() {
                let fraction = (chunk_idx as f64 / num_chunks as f64).clamp(0.0, 1.0);
                cb(fraction, Some("Resuming from saved segments".to_string()));
            }
        }

        // Announce the incoming chunk so the UI can label the active step.
        if let Some(cb) = on_progress.as_ref() {
            let fraction = (chunk_idx as f64 / num_chunks as f64).clamp(0.0, 1.0);
            cb(
                fraction,
                Some(format!(
                    "Analyzing batch {} of {}",
                    chunk_idx + 1,
                    num_chunks
                )),
            );
        }

        // Bound the chunk strictly between the first frame and the first frame of the next chunk.
        // For the final chunk, allow up to chunk_last_ts + buffer (no hard upper bound known here).
        //
        // When `all_segments` is non-empty (either prior chunks succeeded OR
        // we seeded with `resume_segments`), chunk_start_time is the last
        // segment's end — so the AI can't emit content that overlaps what we
        // already have. Only fall back to 0.0 for a truly fresh chunk 0.
        let chunk_start_time = all_segments
            .last()
            .map(|s| s.end_seconds)
            .unwrap_or(if chunk_idx == 0 { 0.0 } else { chunk_first_ts });
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

        let chunk_label = format!("Analyzing batch {} of {num_chunks}", chunk_idx + 1);
        let chunk_fraction = (chunk_idx as f64 / num_chunks as f64).clamp(0.0, 1.0);
        let response = with_heartbeat(
            &on_progress,
            chunk_fraction,
            chunk_label,
            generate_with_retry(provider, system_prompt, chunk_message),
        )
        .await?;

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
                speech_rate_report: None,
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
            // Emit before moving into the accumulator so callers see each new
            // segment in order. Kept minimal — callers are expected to be cheap
            // (sending over a channel).
            if let Some(cb) = on_segment.as_ref() {
                cb(&seg);
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

        // Close out this chunk. No message — the UI keeps the "Analyzing
        // batch X of N" label until the next chunk starts.
        if let Some(cb) = on_progress.as_ref() {
            let fraction = ((chunk_idx + 1) as f64 / num_chunks as f64).clamp(0.0, 1.0);
            cb(fraction, None);
        }
    }

    if skipped_chunks > 0 {
        tracing::info!(
            "Resumed generation: skipped {} of {} chunks covered by {} prior segment(s)",
            skipped_chunks,
            num_chunks,
            all_segments.len()
        );
    }

    // Build the final merged script. If every chunk was skipped because
    // resume_segments already covers the whole video, we still have a valid
    // script — fabricate a minimal header so the caller gets back the existing
    // segments rather than an error.
    if merged_script.is_none() && !all_segments.is_empty() {
        merged_script = Some(NarrationScript {
            title: String::new(),
            total_duration_seconds: all_segments.last().map(|s| s.end_seconds).unwrap_or(0.0),
            segments: Vec::new(),
            metadata: ScriptMetadata::default(),
            speech_rate_report: None,
        });
    }

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
fn clamp_chunk_segments(segments: Vec<Segment>, chunk_start: f64, chunk_end: f64) -> Vec<Segment> {
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
pub fn normalize_timeline(mut segments: Vec<Segment>, video_duration: f64) -> Vec<Segment> {
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
pub fn merge_short_segments(segments: Vec<Segment>, min_duration: f64) -> Vec<Segment> {
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

/// Prepend a retry-feedback text block to the user_message content. The
/// feedback lists the segments that overflowed their word budget on the
/// previous attempt — giving the model a concrete correction target without
/// changing the system prompt.
fn prepend_retry_feedback(user_message: serde_json::Value, feedback: &str) -> serde_json::Value {
    let feedback_block = json!({
        "type": "text",
        "text": format!(
            "--- RETRY FEEDBACK (your previous draft had timing overflow) ---\n{feedback}\n\
             Produce a NEW complete script that fits the word budget in every segment. \
             The full schema and rules from the system prompt still apply.\n\
             --- END RETRY FEEDBACK ---\n"
        )
    });
    match user_message {
        serde_json::Value::Array(mut arr) => {
            arr.insert(0, feedback_block);
            serde_json::Value::Array(arr)
        }
        // Providers that take string messages (rare here — Claude/OpenAI/Gemini
        // all use the array form). Fall back to a text wrapper so we don't lose
        // the original payload.
        other => json!([feedback_block, other]),
    }
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
///
/// Exposed to the command layer so callers can pre-count images and decide
/// whether to spawn the single-call fallback progress timer (it would race
/// real per-chunk ticks on the chunked path).
pub const MAX_FRAMES_PER_CALL: usize = 10;

#[allow(clippy::too_many_arguments)]
pub async fn generate_narration(
    provider: &dyn AiProvider,
    system_prompt: &str,
    user_message: serde_json::Value,
    style: &str,
    language: &str,
    resume_segments: Vec<Segment>,
    on_segment: Option<SegmentCallback>,
    on_progress: Option<ProgressCallback>,
) -> Result<NarrationScript, NarratorError> {
    // First pass.
    let mut script = generate_narration_once(
        provider,
        system_prompt,
        user_message.clone(),
        resume_segments.clone(),
        on_segment.clone(),
        on_progress.clone(),
    )
    .await?;

    // Validate against the per-language speech-rate budget. Attach the report
    // so the Review UI can surface overflow before the user exports.
    let report = crate::script_validator::validate_speech_rate(&script, language);
    let overflow_fraction = crate::script_validator::overflow_fraction(&report);

    tracing::info!(
        "Speech-rate validation: {} segments, {:.0}% overflow (style={}, lang={})",
        report.len(),
        overflow_fraction * 100.0,
        style,
        language
    );

    // One retry when a large share of segments exceed their budget. The LLM
    // gets to see exactly which segments overflowed and by how much — this
    // usually produces a tighter second draft without more prompt tuning.
    const OVERFLOW_RETRY_THRESHOLD: f64 = 0.30;
    if overflow_fraction > OVERFLOW_RETRY_THRESHOLD {
        if let Some(cb) = on_progress.as_ref() {
            cb(0.75, Some("Retrying for tighter narration…".to_string()));
        }
        tracing::warn!(
            "Overflow fraction {:.0}% exceeded {:.0}% threshold — retrying once with feedback",
            overflow_fraction * 100.0,
            OVERFLOW_RETRY_THRESHOLD * 100.0
        );
        let feedback = crate::script_validator::format_retry_feedback(&report, language);
        let retry_message = prepend_retry_feedback(user_message, &feedback);
        // Retry with NO resume_segments: we want the LLM to produce a fully
        // fresh draft that respects the word budget for EVERY segment,
        // including ones the caller had previously resumed from. If we passed
        // the original resume_segments, the chunked path would skip those
        // chunks and leave their overflow unfixed.
        //
        // Retry also runs silently (on_segment = None) so the frontend's live
        // preview doesn't get double-populated with segments from both drafts;
        // the terminal SegmentsReplaced event will carry whichever draft we
        // keep.
        match generate_narration_once(
            provider,
            system_prompt,
            retry_message,
            Vec::new(),
            None,
            on_progress,
        )
        .await
        {
            Ok(retry_script) => {
                let retry_report =
                    crate::script_validator::validate_speech_rate(&retry_script, language);
                let retry_overflow = crate::script_validator::overflow_fraction(&retry_report);
                // Keep whichever draft fits the budget best. A retry that made
                // things worse (rare but possible) shouldn't clobber a better
                // first draft. Strict `<` so ties — retry matched the first
                // draft's overflow fraction — go to the first draft too:
                // the user already saw those segments stream in, and
                // silently swapping in different wording with identical
                // overflow is pure UX churn for zero measurable win.
                if retry_overflow < overflow_fraction {
                    tracing::info!(
                        "Retry improved overflow: {:.0}% → {:.0}%",
                        overflow_fraction * 100.0,
                        retry_overflow * 100.0
                    );
                    let final_report = retry_report;
                    script = retry_script;
                    script.speech_rate_report = Some(final_report);
                    return Ok(script);
                }
                tracing::warn!(
                    "Retry did not improve ({:.0}% vs {:.0}%), keeping first draft",
                    retry_overflow * 100.0,
                    overflow_fraction * 100.0
                );
            }
            Err(e) => {
                tracing::warn!("Overflow retry failed, keeping first draft: {e}");
            }
        }
    }

    script.speech_rate_report = Some(report);
    Ok(script)
}

/// One narration-generation pass. Splits into chunked vs single-call, parses
/// the model output into a `NarrationScript`, and runs the full normalization
/// pipeline. Wrapped by `generate_narration` which handles validate + retry.
#[allow(clippy::too_many_arguments)]
async fn generate_narration_once(
    provider: &dyn AiProvider,
    system_prompt: &str,
    user_message: serde_json::Value,
    resume_segments: Vec<Segment>,
    on_segment: Option<SegmentCallback>,
    on_progress: Option<ProgressCallback>,
) -> Result<NarrationScript, NarratorError> {
    // Check if the message has too many image parts — if so, chunk it
    let parts = user_message.as_array();
    let image_count = parts
        .map(|p| p.iter().filter(|v| v["type"] == "image").count())
        .unwrap_or(0);
    let was_chunked = image_count > MAX_FRAMES_PER_CALL;

    let response_text = if was_chunked {
        generate_chunked(
            provider,
            system_prompt,
            &user_message,
            image_count,
            resume_segments,
            on_segment.clone(),
            on_progress.clone(),
        )
        .await?
    } else {
        // Single-call path: resume doesn't apply (there's only one call). No
        // per-segment streaming happens here — the caller emits a single
        // `SegmentsReplaced` event with the final segments after we return.
        let _ = on_segment.as_ref(); // only used by the chunked path
        if let Some(cb) = on_progress.as_ref() {
            cb(0.05, Some("Generating narration with AI…".to_string()));
        }
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
        script.segments.last().map(|s| s.end_seconds).unwrap_or(0.0) + 60.0
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
        // Wrap the polish call in a heartbeat — it can take 10-30s on a long
        // script and is the silent tail right before the final "Complete"
        // label. Anchor at fraction 0.98 so the bar sits near the end while
        // ticking; real completion bumps it to 1.0 via the caller.
        let polish_label = "Polishing narration".to_string();
        match with_heartbeat(
            &on_progress,
            0.98,
            polish_label,
            polish_script(provider, &script, 2.5),
        )
        .await
        {
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

/// One mismatch reported by the critique pass: `segment_index` is the index
/// of the offending segment in the script, `suggestion` is the model's
/// concrete rewrite guidance (fed verbatim into `refine_segment` as the
/// instruction).
#[derive(Debug, Clone)]
struct Mismatch {
    segment_index: usize,
    suggestion: String,
}

/// Run up to 2 critique+refine iterations on `script`, using a handful of
/// representative frames as ground truth. Each iteration asks the model
/// whether each segment's narration matches what's visible at or near its
/// timestamp; any flagged segment is rewritten via `refine_segment` using
/// the critique's own suggestion as the instruction.
///
/// Returns the (possibly updated) script. Never fails the whole pipeline —
/// any critique-side error downgrades to "skip critique, return as-is".
///
/// Cost envelope: one multimodal critique call per iteration + one
/// text-only `refine_segment` call per mismatch per iteration. Default-off,
/// gated by `GenerationParams::strict_mode`.
pub async fn self_critique_and_refine(
    provider: &dyn AiProvider,
    script: NarrationScript,
    frames: &[Frame],
    on_segment: Option<SegmentCallback>,
    on_progress: Option<ProgressCallback>,
) -> NarrationScript {
    const MAX_ITERATIONS: usize = 2;
    const MAX_REFINES_PER_ITER: usize = 5;

    let mut script = script;
    let sample = pick_critique_frames(frames);
    if sample.is_empty() {
        tracing::info!("self-critique skipped: no frames available");
        return script;
    }

    for iter in 0..MAX_ITERATIONS {
        if let Some(cb) = on_progress.as_ref() {
            cb(
                0.97,
                Some(format!(
                    "Self-critique pass {} of {}",
                    iter + 1,
                    MAX_ITERATIONS
                )),
            );
        }
        let mismatches = match run_critique(provider, &script, &sample).await {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("self-critique call failed, skipping: {e}");
                return script;
            }
        };
        if mismatches.is_empty() {
            tracing::info!(
                "self-critique iteration {}: no mismatches found, stopping",
                iter + 1
            );
            break;
        }
        tracing::info!(
            "self-critique iteration {}: {} mismatches flagged",
            iter + 1,
            mismatches.len()
        );

        // Bound per-iteration refine calls so a runaway critique with dozens
        // of suggested edits can't run up a surprise bill.
        for mismatch in mismatches.into_iter().take(MAX_REFINES_PER_ITER) {
            let Some(segment) = script.segments.get(mismatch.segment_index) else {
                continue;
            };
            let context = surrounding_context(&script.segments, mismatch.segment_index, 1);
            let instruction = format!(
                "The narration does not match the on-screen content. Fix: {}",
                mismatch.suggestion
            );
            match refine_segment(provider, &segment.text, &instruction, &context).await {
                Ok(new_text) => {
                    if let Some(seg) = script.segments.get_mut(mismatch.segment_index) {
                        seg.text = new_text;
                        if let Some(cb) = on_segment.as_ref() {
                            cb(seg);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "refine_segment failed for index {}: {e}",
                        mismatch.segment_index
                    );
                }
            }
        }
    }

    script
}

/// Pick up to 3 frames at representative positions (start, middle, end
/// quarters) so the critique sees a coherent sample of the timeline
/// without sending the full frame set back to the API.
fn pick_critique_frames(frames: &[Frame]) -> Vec<Frame> {
    if frames.is_empty() {
        return Vec::new();
    }
    if frames.len() <= 3 {
        return frames.to_vec();
    }
    let n = frames.len();
    let indices = [n / 10, n / 2, (n * 8) / 10];
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(3);
    for idx in indices {
        let idx = idx.min(n - 1);
        if seen.insert(idx) {
            out.push(frames[idx].clone());
        }
    }
    out
}

/// Build a context string containing the `window` segments before and after
/// `idx`, used as the context for `refine_segment`.
fn surrounding_context(segments: &[Segment], idx: usize, window: usize) -> String {
    let start = idx.saturating_sub(window);
    let end = (idx + window + 1).min(segments.len());
    segments[start..end]
        .iter()
        .enumerate()
        .filter_map(|(offset, seg)| {
            let abs_idx = start + offset;
            if abs_idx == idx {
                None
            } else {
                Some(format!(
                    "[{:.1}s] {}",
                    seg.start_seconds,
                    truncate_chars(&seg.text, 300)
                ))
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn run_critique(
    provider: &dyn AiProvider,
    script: &NarrationScript,
    sample_frames: &[Frame],
) -> Result<Vec<Mismatch>, NarratorError> {
    let system_prompt = "You are reviewing a narration script against sampled video frames. \
        For each narration segment whose text materially contradicts or misses what is \
        visible in the closest frame, list one mismatch. Ignore minor wording issues — \
        only flag real disagreements with the visible content. If the script looks correct, \
        return an empty array. Respond with ONLY a JSON object of the form \
        {\"mismatches\": [{\"segment_index\": <int>, \"reason\": \"<why>\", \"suggestion\": \"<concrete rewrite guidance>\"}]} \
        — no markdown, no commentary.";

    // Compact the script into a single plain-text listing so the model can
    // reason about timestamps without also having to parse our full JSON
    // schema. Clip per-segment text to keep the prompt bounded on long
    // scripts.
    let script_text: String = script
        .segments
        .iter()
        .map(|s| {
            format!(
                "[{}] {:.1}s–{:.1}s: {}",
                s.index,
                s.start_seconds,
                s.end_seconds,
                truncate_chars(&s.text, 400)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let mut content: Vec<serde_json::Value> = Vec::new();
    content.push(json!({
        "type": "text",
        "text": format!("Narration script (index, window, text):\n{script_text}\n\nSampled frames follow. Match each frame against the segment whose window contains its timestamp."),
    }));
    for frame in sample_frames {
        if !frame.path.exists() {
            continue;
        }
        let b64 = video_engine::frame_to_base64(&frame.path)?;
        content.push(json!({
            "type": "text",
            "text": format!("[Sample frame at {:.1}s]", frame.timestamp_seconds),
        }));
        content.push(json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": "image/jpeg",
                "data": b64,
            }
        }));
    }

    let user_message = serde_json::Value::Array(content);
    let response = provider.generate(system_prompt, user_message).await?;
    parse_critique_response(&response)
}

/// Parse a critique JSON response tolerant of code fences / stray text.
fn parse_critique_response(raw: &str) -> Result<Vec<Mismatch>, NarratorError> {
    let trimmed = raw
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    // Extract the first {...} block so leading/trailing prose is tolerated.
    let start = trimmed.find('{').unwrap_or(0);
    let end = trimmed.rfind('}').map(|i| i + 1).unwrap_or(trimmed.len());
    let slice = &trimmed[start..end];
    let value: serde_json::Value = serde_json::from_str(slice)
        .map_err(|e| NarratorError::ApiError(format!("critique JSON parse failed: {e}")))?;
    let arr = value
        .get("mismatches")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::with_capacity(arr.len());
    for entry in arr {
        let Some(idx) = entry.get("segment_index").and_then(|v| v.as_u64()) else {
            continue;
        };
        let suggestion = entry
            .get("suggestion")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if suggestion.is_empty() {
            continue;
        }
        out.push(Mismatch {
            segment_index: idx as usize,
            suggestion,
        });
    }
    Ok(out)
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

/// Whole-script AI refinement driven by a user instruction.
///
/// Where `refine_segment` edits one segment in isolation and `polish_script`
/// is an unattended quality pass, this is the user-driven version: the editor
/// asks for something specific ("make it more technical", "cut 30%", "use
/// second person") and the model rewrites the ENTIRE script while respecting:
///   - The user's instruction as the primary directive
///   - The project's narration style (professional voice, pacing)
///   - The target language (localization stays consistent)
///   - Visual descriptions and frame references (so the narrative stays
///     grounded in what's actually on screen)
///   - Timestamps (don't drift the timeline — segments keep their slots)
///   - The optional project-wide custom prompt (user's global steering)
pub async fn refine_script(
    provider: &dyn AiProvider,
    script: &NarrationScript,
    instruction: &str,
    style_hint: &str,
    custom_prompt: Option<&str>,
) -> Result<NarrationScript, NarratorError> {
    if instruction.trim().is_empty() {
        return Err(NarratorError::ApiError(
            "Instruction is required for whole-script refinement".into(),
        ));
    }

    let language = if script.metadata.language.is_empty() {
        "the script's current language"
    } else {
        script.metadata.language.as_str()
    };

    let mut system_prompt = format!(
        "You are a professional narration script editor. You are given an ENTIRE \
         timed narration script as JSON plus a user instruction describing how \
         to rewrite it. Rewrite the script holistically to satisfy the instruction \
         while respecting these invariants:\n\
         \n\
         TIMELINE\n\
         - Keep the same number of segments unless the instruction explicitly \
           asks you to merge, split, or remove some.\n\
         - Preserve each segment's start_seconds and end_seconds so the narration \
           stays synchronized with the video. Only change timestamps when merging \
           or splitting, and do so by combining/dividing the original ranges.\n\
         - Segments must remain in strictly ascending time order with no overlap.\n\
         \n\
         CONTENT\n\
         - Rewrite `text` to follow the user instruction.\n\
         - Stay grounded in what the video shows — each segment's \
           `visual_description` and `frame_refs` describe what is on screen \
           during that slot. Do not invent content not supported by the visuals.\n\
         - Keep factual claims, product names, and API/command/code snippets \
           accurate; only change phrasing, tone, or structure as instructed.\n\
         - Preserve [pause] markers unless removing them is part of the instruction.\n\
         \n\
         STYLE\n\
         - Narration style: {style_hint}\n\
         - Target language: {language}. Respond in the SAME language as the input \
           text. Do not translate unless the instruction explicitly asks for translation.\n"
    );
    if let Some(p) = custom_prompt {
        let trimmed = p.trim();
        if !trimmed.is_empty() {
            system_prompt.push_str(&format!(
                "\nPROJECT STEERING (applies to every pass)\n{}\n",
                trimmed
            ));
        }
    }
    system_prompt.push_str(
        "\nOUTPUT\n\
         Respond with ONLY valid JSON in the exact same schema as the input \
         (top-level keys: title, total_duration_seconds, segments, metadata). \
         Each segment must retain its index, start_seconds, end_seconds, \
         visual_description, emphasis, pace, pause_after_ms, and frame_refs fields. \
         No markdown code fences. No prose. No explanation.",
    );

    // Serialize the input script compactly — no need for pretty-print inside
    // the prompt.
    let script_json = serde_json::to_string(script)
        .map_err(|e| NarratorError::SerializationError(e.to_string()))?;
    let user_message = json!(format!(
        "INSTRUCTION:\n{}\n\nCURRENT SCRIPT (JSON):\n{}",
        instruction.trim(),
        script_json
    ));

    let response_text = generate_with_retry(provider, &system_prompt, user_message).await?;
    let json_text = response_text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let mut refined: NarrationScript = serde_json::from_str(json_text).map_err(|e| {
        NarratorError::ApiError(format!(
            "Whole-script refinement returned invalid JSON: {e}\nResponse: {}",
            truncate_chars(json_text, 500)
        ))
    })?;

    // Preserve metadata identity (language, provider, generated_at) — a
    // content refactor shouldn't relabel them.
    refined.metadata = script.metadata.clone();
    if refined.total_duration_seconds <= 0.0 {
        refined.total_duration_seconds = script.total_duration_seconds;
    }
    if refined.title.is_empty() {
        refined.title = script.title.clone();
    }

    // Normalize + ensure sane durations as a safety net — the AI occasionally
    // returns overlapping or out-of-order ranges.
    let duration = if refined.total_duration_seconds > 0.0 {
        refined.total_duration_seconds
    } else {
        script.total_duration_seconds
    };
    refined.segments = normalize_timeline(std::mem::take(&mut refined.segments), duration);

    Ok(refined)
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
    fn critique_parse_accepts_bare_json() {
        let raw = r#"{"mismatches":[{"segment_index":0,"reason":"wrong","suggestion":"fix it"}]}"#;
        let out = parse_critique_response(raw).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].segment_index, 0);
        assert_eq!(out[0].suggestion, "fix it");
    }

    #[test]
    fn critique_parse_strips_code_fences() {
        let raw =
            "```json\n{\"mismatches\":[{\"segment_index\":2,\"suggestion\":\"rewrite\"}]}\n```";
        let out = parse_critique_response(raw).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].segment_index, 2);
    }

    #[test]
    fn critique_parse_tolerates_prose_prefix() {
        let raw = "Sure! Here is the JSON:\n{\"mismatches\":[{\"segment_index\":1,\"suggestion\":\"x\"}]}\nLet me know if you need more.";
        let out = parse_critique_response(raw).unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn critique_parse_empty_mismatches_is_ok() {
        let raw = r#"{"mismatches":[]}"#;
        let out = parse_critique_response(raw).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn critique_parse_drops_entries_with_empty_suggestion() {
        let raw = r#"{"mismatches":[{"segment_index":0,"suggestion":""},{"segment_index":1,"suggestion":"real fix"}]}"#;
        let out = parse_critique_response(raw).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].segment_index, 1);
    }

    #[test]
    fn critique_parse_invalid_json_errors() {
        let raw = "not even close to json";
        assert!(parse_critique_response(raw).is_err());
    }

    #[test]
    fn pick_critique_frames_caps_at_three() {
        let mk = |i: usize, t: f64| Frame {
            index: i,
            timestamp_seconds: t,
            path: std::path::PathBuf::from("/dev/null"),
            width: 0,
            height: 0,
        };
        let frames: Vec<Frame> = (0..20).map(|i| mk(i, i as f64)).collect();
        let picked = pick_critique_frames(&frames);
        assert!(picked.len() <= 3);
        // Covers start-ish, middle, end-ish.
        assert!(picked.first().unwrap().index < 5);
        assert!(picked.last().unwrap().index >= 10);
    }

    #[test]
    fn pick_critique_frames_returns_all_when_small() {
        let mk = |i: usize| Frame {
            index: i,
            timestamp_seconds: i as f64,
            path: std::path::PathBuf::from("/dev/null"),
            width: 0,
            height: 0,
        };
        let frames: Vec<Frame> = (0..2).map(mk).collect();
        let picked = pick_critique_frames(&frames);
        assert_eq!(picked.len(), 2);
    }

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

        let prompt = build_system_prompt(&style, &docs, "Focus on the UI elements.", "en");
        assert!(prompt.contains("technical video"));
        assert!(prompt.contains("glossary.md"));
        assert!(prompt.contains("Focus on the UI elements"));
        assert!(prompt.contains("JSON"));
        assert!(
            prompt.contains("WORD BUDGET") && prompt.contains("150"),
            "expected word-budget section mentioning 150 wpm"
        );
    }

    #[test]
    fn test_build_system_prompt_japanese_uses_chars() {
        let style = NarrationStyle {
            id: "x".into(),
            label: "x".into(),
            description: "x".into(),
            system_prompt: "x".into(),
            pacing: "medium".into(),
            pause_markers: false,
        };
        let prompt = build_system_prompt(&style, &[], "", "ja");
        assert!(prompt.contains("characters"));
        assert!(prompt.contains("400"));
    }

    #[test]
    fn prepend_retry_feedback_inserts_at_front_of_array() {
        let user_message = json!([
            {"type": "text", "text": "original"},
            {"type": "image", "source": {"data": "..."}},
        ]);
        let with_feedback = prepend_retry_feedback(user_message, "shorten segment 0");
        let arr = with_feedback.as_array().expect("must stay an array");
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0]["type"], "text");
        assert!(arr[0]["text"].as_str().unwrap().contains("RETRY FEEDBACK"));
        assert!(arr[0]["text"]
            .as_str()
            .unwrap()
            .contains("shorten segment 0"));
        // Original content still there, in order
        assert_eq!(arr[1]["text"], "original");
        assert_eq!(arr[2]["type"], "image");
    }

    #[test]
    fn prepend_retry_feedback_wraps_non_array() {
        // Shouldn't happen in practice (all providers use array form) but the
        // helper must not drop the original payload.
        let user_message = json!("just a string");
        let with_feedback = prepend_retry_feedback(user_message, "fb");
        let arr = with_feedback.as_array().expect("wrapped into array");
        assert_eq!(arr.len(), 2);
        assert!(arr[0]["text"].as_str().unwrap().contains("fb"));
        assert_eq!(arr[1], "just a string");
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
            vec![],
            None,
            None,
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
            vec![],
            None,
            None,
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

        let result = generate_narration(
            &mock,
            "system prompt",
            json!("user message"),
            "test",
            "en",
            vec![],
            None,
            None,
        )
        .await;

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
        let segs = vec![seg(0, 10.0, 10.2, "too short"), seg(1, 30.0, 40.0, "fine")];
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
        assert_eq!(
            out.len(),
            2,
            "got {:?}",
            out.iter()
                .map(|s| (s.start_seconds, s.end_seconds, s.text.clone()))
                .collect::<Vec<_>>()
        );
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
                responses: std::sync::Mutex::new(responses.into_iter().map(String::from).collect()),
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
        let provider = ScriptedProvider::new(vec![&r1, &r2, &r3, &polish_response]);

        let msg = user_msg_with_frames(25, 1.0);
        let result = generate_narration(&provider, "sys", msg, "test", "en", vec![], None, None)
            .await
            .unwrap();

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
            &[(5.0, 8.0, "backwards jump!"), (22.0, 28.0, "later")],
        );
        let provider = ScriptedProvider::new(vec![&r1, &r2]);
        let msg = user_msg_with_frames(15, 2.0); // 2 chunks

        let result = generate_narration(&provider, "sys", msg, "test", "en", vec![], None, None)
            .await
            .unwrap();

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

        generate_narration(&provider, "sys", msg, "test", "en", vec![], None, None)
            .await
            .unwrap();

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
            &[(0.0, 5.0, "valid"), (50.0, 60.0, "wildly out of range")],
        );
        let r2 = chunk_response_json("T", 30.0, &[(15.0, 20.0, "chunk2")]);
        let provider = ScriptedProvider::new(vec![&r1, &r2]);
        let msg = user_msg_with_frames(15, 2.0);

        let result = generate_narration(&provider, "sys", msg, "test", "en", vec![], None, None)
            .await
            .unwrap();

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

        let result = generate_narration(&provider, "sys", msg, "test", "en", vec![], None, None)
            .await
            .unwrap();

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

        generate_narration(&provider, "sys", msg, "test", "en", vec![], None, None)
            .await
            .unwrap();

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
        let r1 = chunk_response_json("T", 30.0, &[(0.0, 10.0, &long_jp), (10.0, 20.0, &long_jp)]);
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

        let result = generate_narration(&provider, "sys", msg, "test", "en", vec![], None, None)
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

        let result = generate_narration(&provider, "sys", msg, "test", "en", vec![], None, None)
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

        let result = generate_narration(&provider, "sys", msg, "test", "ja", vec![], None, None)
            .await
            .unwrap();
        assert_eq!(result.segments.len(), 3);
        assert_eq!(result.segments[0].text, "セグメント 1 🎬");
    }

    #[tokio::test]
    async fn test_single_call_at_max_frames_threshold() {
        // Exactly MAX_FRAMES_PER_CALL frames → single-call path, not chunked
        let resp = chunk_response_json("T", 30.0, &[(0.0, 15.0, "a"), (15.0, 30.0, "b")]);
        let provider = ScriptedProvider::new(vec![&resp]);
        let msg = user_msg_with_frames(MAX_FRAMES_PER_CALL, 1.0);

        generate_narration(&provider, "sys", msg, "test", "en", vec![], None, None)
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

        generate_narration(&provider, "sys", msg, "test", "en", vec![], None, None)
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

        let result =
            generate_narration(&provider, "sys", msg, "test", "en", vec![], None, None).await;
        assert!(
            result.is_ok(),
            "should still succeed with unparseable timestamps"
        );
    }

    #[tokio::test]
    async fn test_chunked_generation_invalid_json_chunk_errors_cleanly() {
        let r1 = chunk_response_json("T", 30.0, &[(0.0, 5.0, "ok")]);
        // Chunk 2 returns garbage — should bubble up a clear ApiError
        let provider = ScriptedProvider::new(vec![&r1, "NOT JSON AT ALL"]);
        let msg = user_msg_with_frames(15, 2.0);

        let err = generate_narration(&provider, "sys", msg, "test", "en", vec![], None, None)
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

        generate_narration(&provider, "sys", msg, "test", "en", vec![], None, None)
            .await
            .unwrap();
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

        let result = generate_narration(&provider, "sys", msg, "test", "en", vec![], None, None)
            .await
            .unwrap();
        // Should succeed with empty segments — not crash
        assert_eq!(result.segments.len(), 0);
    }

    type ProgressTicks = Arc<std::sync::Mutex<Vec<(f64, Option<String>)>>>;

    #[tokio::test]
    async fn test_chunked_progress_callback_fires_bounds() {
        // 25 frames → 3 chunks. Progress callback must fire at least once
        // near 0.0 (before chunk 1) and once near 1.0 (after chunk 3), so the
        // UI bar smoothly traverses the narration slice.
        let r1 = chunk_response_json("T", 25.0, &[(0.0, 5.0, "a"), (5.0, 10.0, "b")]);
        let r2 = chunk_response_json("T", 25.0, &[(10.0, 15.0, "c"), (15.0, 20.0, "d")]);
        let r3 = chunk_response_json("T", 25.0, &[(20.0, 25.0, "e")]);
        let polish = chunk_response_json(
            "T",
            25.0,
            &[
                (0.0, 5.0, "a"),
                (5.0, 10.0, "b"),
                (10.0, 15.0, "c"),
                (15.0, 20.0, "d"),
                (20.0, 25.0, "e"),
            ],
        );
        let provider = ScriptedProvider::new(vec![&r1, &r2, &r3, &polish]);
        let msg = user_msg_with_frames(25, 1.0);

        let captured: ProgressTicks = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = captured.clone();
        let cb: ProgressCallback = Arc::new(move |f, m| {
            sink.lock().unwrap().push((f, m));
        });

        generate_narration(&provider, "sys", msg, "test", "en", vec![], None, Some(cb))
            .await
            .unwrap();

        let ticks = captured.lock().unwrap();
        assert!(!ticks.is_empty(), "no progress ticks were captured");

        // First tick: start of chunk 1 → fraction 0.0, label describes batch 1/3.
        let (first_frac, first_msg) = &ticks[0];
        assert!(
            (*first_frac - 0.0).abs() < 1e-6,
            "first tick should start at 0.0, got {first_frac}"
        );
        assert_eq!(first_msg.as_deref(), Some("Analyzing batch 1 of 3"));

        // Last tick: after chunk 3 → fraction 1.0.
        let (last_frac, _) = ticks.last().unwrap();
        assert!(
            (*last_frac - 1.0).abs() < 1e-6,
            "last tick should reach 1.0, got {last_frac}"
        );

        // Each chunk emits (start_msg, end_none) so we should see all
        // three "Analyzing batch X of 3" labels in order.
        let labels: Vec<&str> = ticks.iter().filter_map(|(_, m)| m.as_deref()).collect();
        assert!(labels.iter().any(|l| l.contains("batch 1 of 3")));
        assert!(labels.iter().any(|l| l.contains("batch 2 of 3")));
        assert!(labels.iter().any(|l| l.contains("batch 3 of 3")));
    }

    #[tokio::test]
    async fn test_chunked_progress_resume_jumps_forward() {
        // With resume_segments covering the first 2/3 chunks, the bar must
        // jump straight to ~0.667 before the live chunk starts — not rebuild
        // from 0 and re-bill the skipped chunks.
        let r3 = chunk_response_json("T", 25.0, &[(20.0, 25.0, "e")]);
        // Single live chunk + polish.
        let polish = chunk_response_json(
            "T",
            25.0,
            &[
                (0.0, 5.0, "a"),
                (5.0, 10.0, "b"),
                (10.0, 15.0, "c"),
                (15.0, 20.0, "d"),
                (20.0, 25.0, "e"),
            ],
        );
        let provider = ScriptedProvider::new(vec![&r3, &polish]);

        let msg = user_msg_with_frames(25, 1.0);
        let resume = vec![
            Segment {
                index: 0,
                start_seconds: 0.0,
                end_seconds: 5.0,
                text: "a".into(),
                visual_description: String::new(),
                emphasis: vec![],
                pace: crate::models::Pace::Medium,
                pause_after_ms: 0,
                frame_refs: vec![],
                voice_override: None,
            },
            Segment {
                index: 1,
                start_seconds: 5.0,
                end_seconds: 10.0,
                text: "b".into(),
                visual_description: String::new(),
                emphasis: vec![],
                pace: crate::models::Pace::Medium,
                pause_after_ms: 0,
                frame_refs: vec![],
                voice_override: None,
            },
            Segment {
                index: 2,
                start_seconds: 10.0,
                end_seconds: 20.0,
                text: "cd".into(),
                visual_description: String::new(),
                emphasis: vec![],
                pace: crate::models::Pace::Medium,
                pause_after_ms: 0,
                frame_refs: vec![],
                voice_override: None,
            },
        ];

        let captured: ProgressTicks = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = captured.clone();
        let cb: ProgressCallback = Arc::new(move |f, m| {
            sink.lock().unwrap().push((f, m));
        });

        generate_narration(&provider, "sys", msg, "test", "en", resume, None, Some(cb))
            .await
            .unwrap();

        let ticks = captured.lock().unwrap();
        // The first emitted tick should be the resume-cutoff jump, not 0.0.
        let (first_frac, first_msg) = &ticks[0];
        assert!(
            *first_frac > 0.5,
            "resume jump should land in the second half, got {first_frac}"
        );
        assert_eq!(first_msg.as_deref(), Some("Resuming from saved segments"));
    }

    // ── translate_script ───────────────────────────────────────────────

    fn translation_response_json(title: &str, lang: &str, segs: &[(f64, f64, &str)]) -> String {
        chunk_response_json(title, 30.0, segs)
            // Override language in metadata (simulate the AI returning the translated script)
            .replacen(
                "\"language\":\"en\"",
                &format!("\"language\":\"{lang}\""),
                1,
            )
    }

    #[tokio::test]
    async fn test_translate_script_success() {
        let original = NarrationScript {
            title: "Original".into(),
            total_duration_seconds: 30.0,
            segments: vec![Segment {
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
            }],
            metadata: ScriptMetadata {
                style: "test".into(),
                language: "en".into(),
                provider: "mock".into(),
                model: "mock-v1".into(),
                generated_at: "2026-04-01T00:00:00Z".into(),
            },
            speech_rate_report: None,
        };
        let resp = translation_response_json("Original", "ja", &[(0.0, 15.0, "こんにちは世界")]);
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
            speech_rate_report: None,
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
            speech_rate_report: None,
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
        let result = refine_segment(
            &provider,
            "original text",
            "make shorter",
            "surrounding context",
        )
        .await
        .unwrap();
        assert_eq!(result, "This is the refined text.");
    }

    #[tokio::test]
    async fn test_refine_segment_strips_quotes_and_fences() {
        let provider = ScriptedProvider::new(vec!["```\n\"Quoted refinement\"\n```"]);
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
        let provider = ScriptedProvider::new(vec!["精緻化されたセグメント 🎬"]);
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
            speech_rate_report: None,
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
        let result = polish_script(&provider, &sample_script(), 2.5)
            .await
            .unwrap();
        assert_eq!(result.title, "Test");
        assert_eq!(result.total_duration_seconds, 30.0);
    }

    // ── refine_script ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_refine_script_rewrites_whole() {
        let resp = chunk_response_json(
            "Test",
            30.0,
            &[(0.0, 3.0, "Tight first."), (3.0, 10.0, "Tight second.")],
        );
        let provider = ScriptedProvider::new(vec![&resp]);
        // NOTE: input has 3 segments; AI returns 2 — intentional consolidation.
        let result = refine_script(
            &provider,
            &sample_script(),
            "Make it more concise.",
            "professional narration",
            None,
        )
        .await
        .unwrap();
        assert_eq!(result.segments.len(), 2);
        assert!(result.segments[0].text.contains("Tight"));
    }

    #[tokio::test]
    async fn test_refine_script_requires_instruction() {
        let resp = chunk_response_json("Test", 30.0, &[(0.0, 10.0, "x")]);
        let provider = ScriptedProvider::new(vec![&resp]);
        let err = refine_script(&provider, &sample_script(), "   ", "style", None)
            .await
            .unwrap_err();
        assert!(err.to_string().to_lowercase().contains("instruction"));
    }

    #[tokio::test]
    async fn test_refine_script_includes_instruction_and_style_in_prompt() {
        let resp = chunk_response_json("Test", 30.0, &[(0.0, 10.0, "x")]);
        let provider = ScriptedProvider::new(vec![&resp]);
        refine_script(
            &provider,
            &sample_script(),
            "Use second person",
            "technical tutorial",
            None,
        )
        .await
        .unwrap();
        let call = provider.captured();
        assert_eq!(call.len(), 1);
        let user_text = call[0].as_str().unwrap_or("");
        assert!(
            user_text.contains("Use second person"),
            "user message missing instruction: {user_text}"
        );
        // Style hint flows through the system prompt — we can't observe the
        // system prompt directly (mock only records user_message), but we
        // ensure the instruction + current script JSON are packaged together.
        assert!(user_text.contains("CURRENT SCRIPT"));
    }

    #[tokio::test]
    async fn test_refine_script_applies_custom_prompt() {
        // Custom project prompt should be accepted without error and the
        // call should succeed with a valid AI response.
        let resp = chunk_response_json("Test", 30.0, &[(0.0, 10.0, "polished")]);
        let provider = ScriptedProvider::new(vec![&resp]);
        let result = refine_script(
            &provider,
            &sample_script(),
            "Refine",
            "style",
            Some("Always use UiPath product voice"),
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_refine_script_preserves_metadata_identity() {
        let resp = chunk_response_json("NewTitle", 30.0, &[(0.0, 10.0, "one")]);
        let provider = ScriptedProvider::new(vec![&resp]);
        let original = sample_script();
        let result = refine_script(&provider, &original, "do it", "style", None)
            .await
            .unwrap();
        // AI returned "NewTitle" but we preserve metadata from input
        assert_eq!(result.metadata.language, original.metadata.language);
        assert_eq!(result.metadata.provider, original.metadata.provider);
        assert_eq!(result.metadata.generated_at, original.metadata.generated_at);
    }

    #[tokio::test]
    async fn test_refine_script_normalizes_on_out_of_order_response() {
        // AI returns segments out of order → refine_script must sort them.
        let resp = chunk_response_json(
            "Test",
            30.0,
            &[
                (20.0, 28.0, "c (last)"),
                (0.0, 8.0, "a (first)"),
                (10.0, 18.0, "b (middle)"),
            ],
        );
        let provider = ScriptedProvider::new(vec![&resp]);
        let result = refine_script(&provider, &sample_script(), "sort me", "style", None)
            .await
            .unwrap();
        assert_eq!(result.segments.len(), 3);
        assert!(result.segments[0].text.contains("a (first)"));
        assert!(result.segments[2].text.contains("c (last)"));
        for w in result.segments.windows(2) {
            assert!(w[0].end_seconds <= w[1].start_seconds + 0.01);
        }
    }

    #[tokio::test]
    async fn test_refine_script_invalid_json_errors() {
        let provider = ScriptedProvider::new(vec!["not json"]);
        let err = refine_script(&provider, &sample_script(), "inst", "style", None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("invalid JSON"));
    }

    #[tokio::test]
    async fn test_refine_script_strips_code_fences() {
        let inner = chunk_response_json("Test", 30.0, &[(0.0, 10.0, "ok")]);
        let fenced = format!("```json\n{inner}\n```");
        let provider = ScriptedProvider::new(vec![&fenced]);
        assert!(
            refine_script(&provider, &sample_script(), "inst", "style", None)
                .await
                .is_ok()
        );
    }
}
