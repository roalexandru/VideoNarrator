//! Multi-provider AI client supporting Claude and OpenAI for narration generation.

use crate::contact_sheet;
use crate::error::NarratorError;
use crate::http_client;
use crate::models::*;
use crate::response_schema::{self, ResponseSchema};
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

    /// Generate with the response shape enforced by the provider rather than
    /// requested in prose. Returns the same thing `generate` does — a JSON
    /// string for the caller to deserialize — so downstream parsing is
    /// unchanged and the tolerant parser stays a working fallback.
    ///
    /// The default implementation ignores the schema and delegates, which keeps
    /// providers (and test doubles) that have no native support working as-is.
    async fn generate_with_schema(
        &self,
        system_prompt: &str,
        user_message: serde_json::Value,
        _schema: &ResponseSchema,
    ) -> Result<String, NarratorError> {
        self.generate(system_prompt, user_message).await
    }

    fn name(&self) -> &str;
    fn model(&self) -> &str;
}

// ── Model capability matrix ──────────────────────────────────────────────────
//
// Frontier models keep *removing* request parameters, so "which knobs may I
// send to this model" is per-model state we have to track. Getting it wrong is
// a hard 400, not a soft degrade. These predicates are the single source of
// truth; the body builders below are pure functions over them so the wire shape
// is unit-testable without touching the network.

/// Claude models that reject `temperature` / `top_p` / `top_k` outright (400).
///
/// Removed starting with Opus 4.7. Sending a sampling parameter to any of these
/// fails the whole request, so the builder must omit it rather than clamp it.
pub fn claude_rejects_sampling_params(model: &str) -> bool {
    model.starts_with("claude-opus-5")
        || model.starts_with("claude-sonnet-5")
        || model.starts_with("claude-fable-5")
        || model.starts_with("claude-mythos-5")
        || model.starts_with("claude-opus-4-8")
        || model.starts_with("claude-opus-4-7")
}

/// Claude models that take `thinking: {type: "adaptive"}` (the 4.6+ family).
///
/// The older fixed `budget_tokens` form is deprecated on 4.6 and rejected with a
/// 400 from 4.7 onward, so adaptive is the only form we ever send.
pub fn claude_supports_adaptive_thinking(model: &str) -> bool {
    model.starts_with("claude-opus-5")
        || model.starts_with("claude-sonnet-5")
        || model.starts_with("claude-fable-5")
        || model.starts_with("claude-mythos-5")
        || model.starts_with("claude-opus-4-8")
        || model.starts_with("claude-opus-4-7")
        || model.starts_with("claude-opus-4-6")
        || model.starts_with("claude-sonnet-4-6")
}

/// Claude models that accept `output_config.effort`.
///
/// Errors on Sonnet 4.5 and Haiku 4.5, so it can't be sent unconditionally.
pub fn claude_supports_effort(model: &str) -> bool {
    claude_supports_adaptive_thinking(model) || model.starts_with("claude-opus-4-5")
}

/// Gemini models that take `generationConfig.thinkingLevel` (Gemini 3+).
///
/// Gemini 2.5 uses the older `thinkingBudget`; the two are mutually exclusive in
/// one request, so we only send the new form to models that understand it.
pub fn gemini_supports_thinking_level(model: &str) -> bool {
    model.starts_with("gemini-3")
}

/// Output-token ceiling. Thinking tokens and visible response text share this
/// budget, so a thinking-capable model needs materially more headroom than the
/// 8192 that sufficed when no model reasoned — too tight and the JSON payload
/// gets truncated mid-object and fails the strict parse downstream.
fn max_output_tokens(thinking_enabled: bool) -> u32 {
    if thinking_enabled {
        16000
    } else {
        8192
    }
}

impl ReasoningEffort {
    /// Anthropic `output_config.effort` (`low` | `medium` | `high` | `xhigh` | `max`).
    fn claude_effort(self) -> &'static str {
        match self {
            ReasoningEffort::Fast => "low",
            ReasoningEffort::Balanced => "medium",
            ReasoningEffort::Thorough => "high",
            ReasoningEffort::Max => "max",
        }
    }

    /// OpenAI Chat Completions `reasoning_effort`.
    ///
    /// Note this is the *flat* parameter used by `/v1/chat/completions`; the
    /// Responses API spells the same thing `reasoning: {effort}`.
    fn openai_effort(self) -> &'static str {
        match self {
            ReasoningEffort::Fast => "low",
            ReasoningEffort::Balanced => "medium",
            ReasoningEffort::Thorough => "high",
            ReasoningEffort::Max => "max",
        }
    }

    /// Gemini `thinkingLevel`. Clamped: Gemini's ladder stops at `high`, so
    /// `Max` maps to `high` rather than sending a value the API would reject.
    fn gemini_thinking_level(self) -> &'static str {
        match self {
            ReasoningEffort::Fast => "low",
            ReasoningEffort::Balanced => "medium",
            ReasoningEffort::Thorough | ReasoningEffort::Max => "high",
        }
    }
}

// ── Request body builders (pure) ─────────────────────────────────────────────

/// Build the Anthropic Messages request body.
pub fn build_claude_body(
    model: &str,
    temperature: f32,
    effort: ReasoningEffort,
    system_prompt: &str,
    user_message: serde_json::Value,
) -> serde_json::Value {
    let thinking = claude_supports_adaptive_thinking(model);

    let mut body = json!({
        "model": model,
        "max_tokens": max_output_tokens(thinking),
        "system": system_prompt,
        "messages": [{
            "role": "user",
            "content": user_message
        }]
    });

    // Omit entirely on models that removed sampling params — clamping is not an
    // option, any value is a 400.
    if !claude_rejects_sampling_params(model) {
        body["temperature"] = json!(temperature);
    }

    if thinking {
        // Always adaptive, never `disabled`: on Fable 5 an explicit `disabled`
        // is a 400, on Opus 5 it's rejected above `high` effort, and with
        // thinking off these models sometimes leak `<thinking>` tags into the
        // visible response — which would break the strict JSON parse this app
        // depends on. "Fast" is expressed as low effort instead.
        body["thinking"] = json!({ "type": "adaptive" });
    }

    if claude_supports_effort(model) {
        body["output_config"] = json!({ "effort": effort.claude_effort() });
    }

    body
}

/// Build the OpenAI Chat Completions request body. `user_content` is expected in
/// OpenAI content-part shape.
pub fn build_openai_body(
    model: &str,
    temperature: f32,
    effort: ReasoningEffort,
    system_prompt: &str,
    user_content: serde_json::Value,
) -> serde_json::Value {
    // Reasoning models require `max_completion_tokens` and reject a user-set
    // `temperature` (only the implicit default is accepted).
    let is_reasoning = is_openai_reasoning_model(model);
    let token_key = if is_reasoning {
        "max_completion_tokens"
    } else {
        "max_tokens"
    };

    let mut body = json!({
        "model": model,
        token_key: max_output_tokens(is_reasoning),
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": user_content}
        ]
    });

    if is_reasoning {
        body["reasoning_effort"] = json!(effort.openai_effort());
    } else {
        body["temperature"] = json!(temperature);
    }

    body
}

/// Build the Gemini `generateContent` request body.
pub fn build_gemini_body(
    model: &str,
    temperature: f32,
    effort: ReasoningEffort,
    system_prompt: &str,
    parts: Vec<serde_json::Value>,
) -> serde_json::Value {
    let thinking = gemini_supports_thinking_level(model);

    let mut generation_config = json!({
        "temperature": temperature,
        "maxOutputTokens": max_output_tokens(thinking),
        // Force strict JSON output. Without this, Gemini occasionally emits
        // Python-dict-style responses (single-quoted keys), which fail the
        // strict serde_json parse downstream.
        "responseMimeType": "application/json"
    });

    if thinking {
        generation_config["thinkingLevel"] = json!(effort.gemini_thinking_level());
    }

    json!({
        "contents": [{ "parts": parts }],
        "systemInstruction": { "parts": [{ "text": system_prompt }] },
        "generationConfig": generation_config
    })
}

// ── Schema enforcement (pure) ────────────────────────────────────────────────
//
// Each provider spells "the response must match this shape" differently. These
// mutate an already-built body so the builders above keep their signatures and
// their existing tests, and each transform is independently unit-testable
// without a network call.

/// Force the response through a single-tool call whose `input_schema` is the
/// contract. Anthropic has no `response_format`; a forced tool is the supported
/// way to pin the output shape.
pub fn apply_claude_schema(body: &mut serde_json::Value, schema: &ResponseSchema) {
    body["tools"] = json!([{
        "name": schema.name,
        "description": schema.description,
        "input_schema": schema.schema.clone(),
    }]);
    body["tool_choice"] = json!({ "type": "tool", "name": schema.name });
}

/// Set `response_format` to a strict JSON schema.
///
/// `strict: true` is what makes this a guarantee rather than a hint, and it is
/// also what imposes the constraints the canonical schemas are written to
/// satisfy (all properties required, `additionalProperties: false`).
pub fn apply_openai_schema(body: &mut serde_json::Value, schema: &ResponseSchema) {
    body["response_format"] = json!({
        "type": "json_schema",
        "json_schema": {
            "name": schema.name,
            "strict": true,
            "schema": schema.schema.clone(),
        }
    });
}

/// Set `generationConfig.responseSchema`, converted to Gemini's OpenAPI subset.
///
/// `responseMimeType` is already `application/json` from the builder; the
/// schema narrows that from "some JSON" to "this JSON".
pub fn apply_gemini_schema(body: &mut serde_json::Value, schema: &ResponseSchema) {
    body["generationConfig"]["responseSchema"] = response_schema::to_gemini_dialect(&schema.schema);
}

// ── Claude Provider ──

pub struct ClaudeProvider {
    pub api_key: String,
    pub model: String,
    pub temperature: f32,
    pub reasoning_effort: ReasoningEffort,
}

/// Pull the payload out of an Anthropic Messages response.
///
/// With a forced tool call the JSON arrives as the `input` object of a
/// `tool_use` block, not as text — so it is re-serialized to a string here and
/// callers keep parsing a string either way.
///
/// Falls back to the first text block when no matching `tool_use` is present.
/// That is not merely defensive: a request that stops for `max_tokens` mid-tool
/// can come back without the block, and a clear parse error downstream beats an
/// empty string that looks like a successful empty script.
fn extract_claude_payload(response: &serde_json::Value, tool_name: Option<&str>) -> String {
    let blocks = response["content"].as_array();

    if let (Some(name), Some(blocks)) = (tool_name, blocks) {
        let tool_input = blocks.iter().find_map(|b| {
            if b["type"] == "tool_use" && b["name"] == name {
                Some(&b["input"])
            } else {
                None
            }
        });
        if let Some(input) = tool_input {
            return input.to_string();
        }
        tracing::warn!(
            "Claude returned no `{name}` tool_use block; falling back to text content \
             (stop_reason: {})",
            response["stop_reason"].as_str().unwrap_or("unknown")
        );
    }

    blocks
        .and_then(|blocks| {
            blocks.iter().find_map(|b| {
                if b["type"] == "text" {
                    b["text"].as_str().map(|s| s.to_string())
                } else {
                    None
                }
            })
        })
        .unwrap_or_default()
}

impl ClaudeProvider {
    /// Single POST to the Messages API. `tool_name` is `Some` when the request
    /// forced a tool call and the response should be read from its input.
    async fn send(
        &self,
        body: serde_json::Value,
        tool_name: Option<&str>,
    ) -> Result<String, NarratorError> {
        let client = http_client::shared();

        // Single attempt — retries on 429/529 are handled exactly once,
        // upstream in `generate_with_retry`. Looping here too compounded
        // backoffs (provider-level 2/4s on top of wrapper 5/15/30/60s)
        // and could pin a single AI call in retry-wait for 100s+.
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
            Ok(extract_claude_payload(&response_json, tool_name))
        } else {
            let error_text = resp.text().await.unwrap_or_default();
            tracing::error!("API error ({status}): {}", truncate_chars(&error_text, 400));
            Err(classify_error_response(
                status.as_u16(),
                &error_text,
                "Claude",
                "Anthropic",
            ))
        }
    }
}

#[async_trait]
impl AiProvider for ClaudeProvider {
    async fn generate(
        &self,
        system_prompt: &str,
        user_message: serde_json::Value,
    ) -> Result<String, NarratorError> {
        let body = build_claude_body(
            &self.model,
            self.temperature,
            self.reasoning_effort,
            system_prompt,
            user_message,
        );
        self.send(body, None).await
    }

    async fn generate_with_schema(
        &self,
        system_prompt: &str,
        user_message: serde_json::Value,
        schema: &ResponseSchema,
    ) -> Result<String, NarratorError> {
        let mut body = build_claude_body(
            &self.model,
            self.temperature,
            self.reasoning_effort,
            system_prompt,
            user_message,
        );
        apply_claude_schema(&mut body, schema);
        self.send(body, Some(schema.name)).await
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

/// Best-effort: does this error body describe a billing / credit / quota
/// problem rather than a transient rate limit? Providers surface these
/// inconsistently and often UNDER a 429 — OpenAI returns `insufficient_quota`,
/// Anthropic a low-credit-balance message — so we classify by content, not
/// status alone. A billing problem is permanent until the user acts: treating
/// it as a rate limit shows "wait and try again" and sends users in circles.
fn looks_like_billing_error(body: &str) -> bool {
    let m = body.to_lowercase();
    m.contains("credit balance")
        || m.contains("insufficient_quota")
        || m.contains("insufficient quota")
        || m.contains("insufficient funds")
        || m.contains("insufficient credit")
        || m.contains("out of credits")
        || m.contains("billing")
        || m.contains("payment")
        || m.contains("exceeded your current quota")
        || m.contains("plans & billing")
        || m.contains("purchase credits")
        || m.contains("spending limit")
        || m.contains("spend limit")
        || m.contains("\"billing_error\"")
}

/// Turn a 402/429/529 response into the right error. 402 is always a billing
/// problem; 429/529 are rate limits UNLESS the body reads as billing/quota.
/// Reads the body so the provider's own explanation reaches the user instead
/// of a generic message. `provider` labels the message ("Claude" / "OpenAI" …).
fn classify_rate_or_billing(status: u16, body: &str, provider: &str) -> NarratorError {
    let api_msg = parse_api_error_message(body);
    let detail = if api_msg.is_empty() {
        truncate_chars(body, 240).into_owned()
    } else {
        api_msg
    };
    if status == 402 || looks_like_billing_error(&detail) || looks_like_billing_error(body) {
        let detail = if detail.trim().is_empty() {
            "your account has no usable credit".to_string()
        } else {
            detail
        };
        NarratorError::InsufficientCredit(format!(
            "{provider} rejected the request — {detail} \
             Add credit or fix billing in the provider's console, then try again."
        ))
    } else {
        NarratorError::RateLimited
    }
}

/// Turn any non-success HTTP status + body into the right `NarratorError`.
/// Shared by all three providers so their error semantics stay identical:
///   - billing/credit/quota wording on ANY status → `InsufficientCredit`
///     (Anthropic ships low-credit as a 400, OpenAI as a 429, so the status
///     alone can't decide — classification keys off the body)
///   - a plain 402 → `InsufficientCredit`
///   - a plain 429/529 → `RateLimited` (transient, retryable)
///   - everything else → a generic, provider-labelled `ApiError` with a hint
///
/// `provider` names the product for the message ("Claude"); `key_vendor` names
/// whose key the 401/403 hint should point at ("Anthropic").
fn classify_error_response(
    status: u16,
    body: &str,
    provider: &str,
    key_vendor: &str,
) -> NarratorError {
    // Billing on any status, then plain rate limits, then the generic path.
    if status == 402 || looks_like_billing_error(body) {
        return classify_rate_or_billing(status, body, provider);
    }
    if matches!(status, 429 | 529) {
        return NarratorError::RateLimited;
    }
    let api_msg = parse_api_error_message(body);
    let hint = match status {
        401 | 403 => format!("Check that your {key_vendor} API key is valid."),
        400 => "The request was rejected — usually a model or parameter mismatch.".to_string(),
        _ => "See the details below.".to_string(),
    };
    let detail = if api_msg.is_empty() {
        truncate_chars(body, 240).into_owned()
    } else {
        api_msg
    };
    NarratorError::ApiError(format!(
        "{provider} API error (HTTP {status}). {hint}\n\n{detail}"
    ))
}

pub struct OpenAiProvider {
    pub api_key: String,
    pub model: String,
    pub temperature: f32,
    pub reasoning_effort: ReasoningEffort,
}

/// Translate a Claude-shaped content array into OpenAI content parts.
///
/// The Claude block shape is canonical throughout this module; each provider
/// converts on the way out.
pub fn claude_content_to_openai(user_message: &serde_json::Value) -> serde_json::Value {
    if let Some(parts) = user_message.as_array() {
        let converted: Vec<serde_json::Value> = parts
            .iter()
            .map(|part| {
                if part["type"] == "image" {
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
                    json!({
                        "type": "text",
                        "text": part["text"].as_str().unwrap_or("")
                    })
                }
            })
            .collect();
        serde_json::Value::Array(converted)
    } else if user_message.is_string() {
        json!([{"type": "text", "text": user_message.as_str().unwrap_or("")}])
    } else {
        json!([{"type": "text", "text": user_message.to_string()}])
    }
}

impl OpenAiProvider {
    async fn send(&self, body: serde_json::Value) -> Result<String, NarratorError> {
        let client = http_client::shared();

        // Retries handled by `generate_with_retry`; see ClaudeProvider for the
        // rationale on collapsing the inner loop.
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
            Ok(text)
        } else {
            let error_text = resp.text().await.unwrap_or_default();
            tracing::error!("API error ({status}): {}", truncate_chars(&error_text, 400));
            Err(classify_error_response(
                status.as_u16(),
                &error_text,
                "OpenAI",
                "OpenAI",
            ))
        }
    }
}

#[async_trait]
impl AiProvider for OpenAiProvider {
    async fn generate(
        &self,
        system_prompt: &str,
        user_message: serde_json::Value,
    ) -> Result<String, NarratorError> {
        let body = build_openai_body(
            &self.model,
            self.temperature,
            self.reasoning_effort,
            system_prompt,
            claude_content_to_openai(&user_message),
        );
        self.send(body).await
    }

    async fn generate_with_schema(
        &self,
        system_prompt: &str,
        user_message: serde_json::Value,
        schema: &ResponseSchema,
    ) -> Result<String, NarratorError> {
        let mut body = build_openai_body(
            &self.model,
            self.temperature,
            self.reasoning_effort,
            system_prompt,
            claude_content_to_openai(&user_message),
        );
        apply_openai_schema(&mut body, schema);
        self.send(body).await
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
    pub reasoning_effort: ReasoningEffort,
}

/// Translate a Claude-shaped content array into Gemini parts.
pub fn claude_content_to_gemini(user_message: &serde_json::Value) -> Vec<serde_json::Value> {
    if let Some(parts) = user_message.as_array() {
        parts
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
    }
}

impl GeminiProvider {
    async fn send(&self, body: serde_json::Value) -> Result<String, NarratorError> {
        let client = http_client::shared();

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
            self.model
        );

        // Retries handled by `generate_with_retry`; see ClaudeProvider for the
        // rationale on collapsing the inner loop.
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
            Ok(text)
        } else {
            let error_text = resp.text().await.unwrap_or_default();
            tracing::error!("API error ({status}): {}", truncate_chars(&error_text, 400));
            Err(classify_error_response(
                status.as_u16(),
                &error_text,
                "Gemini",
                "Google",
            ))
        }
    }
}

#[async_trait]
impl AiProvider for GeminiProvider {
    async fn generate(
        &self,
        system_prompt: &str,
        user_message: serde_json::Value,
    ) -> Result<String, NarratorError> {
        let body = build_gemini_body(
            &self.model,
            self.temperature,
            self.reasoning_effort,
            system_prompt,
            claude_content_to_gemini(&user_message),
        );
        self.send(body).await
    }

    async fn generate_with_schema(
        &self,
        system_prompt: &str,
        user_message: serde_json::Value,
        schema: &ResponseSchema,
    ) -> Result<String, NarratorError> {
        let mut body = build_gemini_body(
            &self.model,
            self.temperature,
            self.reasoning_effort,
            system_prompt,
            claude_content_to_gemini(&user_message),
        );
        apply_gemini_schema(&mut body, schema);
        self.send(body).await
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
            reasoning_effort: config.reasoning_effort,
        }),
        AiProviderKind::OpenAi => Box::new(OpenAiProvider {
            api_key,
            model: config.model.clone(),
            temperature: config.temperature,
            reasoning_effort: config.reasoning_effort,
        }),
        AiProviderKind::Gemini => Box::new(GeminiProvider {
            api_key,
            model: config.model.clone(),
            temperature: config.temperature,
            reasoning_effort: config.reasoning_effort,
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

    // Base instructions + output shape.
    //
    // The schema is still spelled out even though `response_schema` now enforces
    // it at the API level: it documents the field *meanings*, which a bare schema
    // does not, and it is the only description available on the tolerant-parse
    // fallback path.
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
        \"generated_at\": \"ISO8601 string\"\n  }\n}\n\n",
    );

    // ── HARD RULES ──
    //
    // Deliberately short. Everything here breaks the export or the TTS engine if
    // violated — these are correctness constraints, not taste. Mixing craft
    // advice in with them (as the previous single "CRITICAL RULES" list did)
    // dilutes both: the model treats a stylistic preference as inviolable and a
    // real constraint as one item among many.
    prompt.push_str(&format!(
        "## HARD RULES (violating any of these breaks the export)\n\n\
        1. Return ONLY the JSON. No markdown fences, no prose before or after.\n\
        2. Segments MUST cover the ENTIRE video, and the last segment's \
           `end_seconds` MUST equal `total_duration_seconds`.\n\
        3. Segments MUST be in ascending time order and MUST NOT overlap.\n\
        4. `text` MUST be plain speakable text. NEVER emit markup, tags, or \
           directives such as [pause], [break], (pause) — the string goes \
           straight to a text-to-speech engine, which will read them aloud.\n\
        5. WORD BUDGET. The engine for language '{lang}' delivers roughly \
           {target_rate:.0} {unit} per minute. For EVERY segment, `text` MUST fit \
           inside its window at that rate:\n\n\
        \tmax_{unit} = round((end_seconds - start_seconds) × {target_rate:.0} / 60)\n\n\
        \tWhen an idea does not fit, do ONE of these — never cram:\n\
        \t• Extend `end_seconds` (borrow from the gap before the next segment).\n\
        \t• Split the idea across two adjacent segments.\n\
        \t• Cut it down — trim adjectives, drop asides.\n\n\
        \tExceeding the budget makes the exported video audibly desync or \
        \tstretch. It is a hard upper bound, not a target.\n\n"
    ));

    // ── CRAFT ──
    //
    // Defaults that work, explicitly marked as the model's call. Stated as
    // guidance rather than rules so the model can depart from them when the
    // material calls for it, instead of following them off a cliff.
    prompt.push_str(
        "## CRAFT (defaults that work — your judgement, not rules)\n\n\
        These are starting points from videos that turned out well. Depart from \
        any of them when the footage justifies it.\n\n\
        • Coverage: speech over roughly 75-85% of the duration. Long silent \
          stretches feel empty; wall-to-wall talking feels breathless.\n\
        • Gaps: 1-3 seconds between segments gives the ear somewhere to rest.\n\
        • Distribution: spread segments across the whole timeline rather than \
          front-loading them.\n\
        • Density: when the budget is generous, be substantive; when it is tight, \
          be brief. A short segment is better than a rushed one.\n\
        • Specificity: name what is actually on screen — the command, the file, \
          the error, the value. Concrete beats generic every time.\n\n",
    );

    // ── AVOID ──
    //
    // Named failure modes observed in real output. Cheaper and more effective
    // than adding another positive rule, because each one is a pattern the model
    // otherwise reaches for by default.
    prompt.push_str(
        "## AVOID (these consistently make narration worse)\n\n\
        • Reading on-screen text aloud verbatim. The viewer can already read it — \
          say what it means or why it matters instead.\n\
        • Filler openers: \"In this video we'll see…\", \"Let's take a look at…\", \
          \"As you can see…\". Start with the substance.\n\
        • Empty intensifiers: \"seamlessly\", \"effortlessly\", \"powerful\", \
          \"robust\", \"simply\". They add syllables and no information.\n\
        • Narrating the interface chrome — menu bars, window titles, the clock, \
          the cursor moving. Describe the work, not the furniture.\n\
        • Narrating the act of clicking or typing when the result is what matters.\n\
        • Cramming past the word budget and hoping the engine keeps up.\n\
        • Restating the previous segment in different words to fill a window.\n\n",
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
    tile: bool,
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

    content.extend(frame_content_parts(frames, tile)?);

    Ok(serde_json::Value::Array(content))
}

/// Frames per contact sheet — one full grid.
pub const FRAMES_PER_SHEET: usize =
    (contact_sheet::DEFAULT_COLUMNS * contact_sheet::DEFAULT_COLUMNS) as usize;

/// Render frames as model content parts.
///
/// `tile == false` is the historical shape: one labelled image per frame.
/// `tile == true` groups them into contact sheets, so nine moments occupy one
/// image slot instead of nine — which is what lets a long video be analysed in a
/// handful of calls instead of thirty, each retaining full context.
fn frame_content_parts(
    frames: &[Frame],
    tile: bool,
) -> Result<Vec<serde_json::Value>, NarratorError> {
    // Missing files are filtered first, preserving the historical
    // skip-silently behaviour for a frame whose file vanished.
    let present: Vec<Frame> = frames.iter().filter(|f| f.path.exists()).cloned().collect();
    let mut parts = Vec::new();

    if tile {
        for group in present.chunks(FRAMES_PER_SHEET) {
            let Some(sheet) = contact_sheet::build(
                group,
                contact_sheet::DEFAULT_COLUMNS,
                contact_sheet::DEFAULT_CELL_WIDTH,
            )?
            else {
                continue;
            };
            // The mapping text must precede the image, or the model reads the
            // grid before being told how to index it.
            parts.push(json!({"type": "text", "text": sheet.describe()}));
            parts.push(json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": "image/jpeg",
                    "data": sheet.base64
                }
            }));
        }
        return Ok(parts);
    }

    // Encoding is downscale + JPEG per frame — pure CPU that ran serially for up
    // to 300 frames, now spread across cores.
    let paths: Vec<std::path::PathBuf> = present.iter().map(|f| f.path.clone()).collect();
    let encoded = crate::frame_cache::encode_frames_parallel(&paths)?;

    for (frame, b64) in present.iter().zip(encoded) {
        parts.push(json!({
            "type": "text",
            "text": format!("[Frame {} at {:.1}s]", frame.index, frame.timestamp_seconds)
        }));
        parts.push(json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": "image/jpeg",
                "data": b64
            }
        }));
    }

    Ok(parts)
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
    video_duration: f64,
    resume_segments: Vec<Segment>,
    on_segment: Option<SegmentCallback>,
    on_progress: Option<ProgressCallback>,
    cancel: Option<Arc<AtomicBool>>,
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
        // The final chunk runs to the end of the video, not just to its last
        // frame. Sampling stops before the end (the last anchor on a 220 s video
        // landed at 210 s), so bounding the chunk at its last frame leaves the
        // tail permanently unnarrated.
        let chunk_end_time = match next_chunk_first_ts {
            Some(next) => next,
            None if video_duration > chunk_last_ts => video_duration,
            None => chunk_last_ts + 30.0,
        };

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
        // Chunks own 0..CHUNK_SPAN of the narration band, leaving headroom for the
        // post-chunk passes (polish, stretch, merge). Previously chunks consumed
        // the whole band, so the bar hit 99% and then sat there for up to 90s
        // while polish ran — its heartbeat reported a *lower* fraction, which the
        // frontend's monotonic clamp discarded entirely.
        let chunk_fraction = (chunk_idx as f64 / num_chunks as f64).clamp(0.0, 1.0) * CHUNK_SPAN;
        // `RetryContext` owns both the in-flight heartbeat and the rate-limit
        // countdown, so we no longer wrap this in `with_heartbeat` — that would
        // repaint "· Ns elapsed" over the countdown message every 1.5s.
        let ctx = on_progress.clone().map(|cb| RetryContext {
            on_progress: cb,
            fraction: chunk_fraction,
            label: chunk_label,
            cancel: cancel.clone(),
        });
        let response = generate_with_retry(
            provider,
            system_prompt,
            chunk_message,
            ctx.as_ref(),
            Some(&response_schema::narration_script()),
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
                chapters: None,
                title: chunk_script.title.clone(),
                // NOT `chunk_script.total_duration_seconds`. The first chunk only
                // sees its own frames and reports that slice's length, which the
                // final `normalize_timeline` then used as a hard cutoff —
                // deleting every later chunk's segments.
                total_duration_seconds: if video_duration > 0.0 {
                    video_duration
                } else {
                    chunk_script.total_duration_seconds
                },
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
            let fraction =
                ((chunk_idx + 1) as f64 / num_chunks as f64).clamp(0.0, 1.0) * CHUNK_SPAN;
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
            chapters: None,
            title: String::new(),
            total_duration_seconds: all_segments.last().map(|s| s.end_seconds).unwrap_or(0.0),
            segments: Vec::new(),
            metadata: ScriptMetadata::default(),
            speech_rate_report: None,
        });
    }

    if let Some(mut script) = merged_script {
        // Final normalization pass (guarantees monotonic, non-overlapping, re-indexed).
        // Bounded by the measured video length — see the header comment above for
        // why the model's figure must not be used here.
        let bound = if video_duration > 0.0 {
            video_duration
        } else {
            script.total_duration_seconds
        };
        let before = all_segments.len();
        let normalized = normalize_timeline(all_segments, bound);
        if normalized.len() < before {
            tracing::warn!(
                "chunked merge: normalize dropped {} of {before} segments against a {bound:.1}s bound",
                before - normalized.len()
            );
        }
        script.segments = normalized;

        // Coverage guard. Silent truncation is the failure mode that shipped a
        // 53 s script for a 220 s video, so make it loud rather than trusting the
        // pipeline to be correct.
        if bound > 0.0 {
            let covered = script.segments.last().map(|s| s.end_seconds).unwrap_or(0.0);
            if covered < bound * 0.6 {
                tracing::error!(
                    "narration covers only {covered:.1}s of a {bound:.1}s video ({:.0}%) \
                     across {} segment(s) — this indicates dropped chunks, not a short script",
                    covered / bound * 100.0,
                    script.segments.len()
                );
            }
        }
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

// ── Silence-aware segment edges ──────────────────────────────────────────────
//
// Segment times are invented by the model and only *normalized* afterwards;
// nothing tied them to anything observable in the source. When the recording
// carries audio of its own — app sounds, background music, a presenter
// mid-demo — narration starts and stops wherever the model guessed, frequently
// on top of that audio.
//
// The `silencedetect` pass already knows where the quiet stretches are. Snapping
// each segment edge into a nearby gap costs nothing extra and makes narration
// land in the holes instead of over the content.

/// Shortest gap a segment edge may be snapped into.
///
/// Below this a gap is mid-phrase — inside a word or between two syllables —
/// and starting to speak there sounds like an interruption.
pub const MIN_SNAP_GAP: f64 = 0.15;

/// Gap width at or above which a snap is unambiguously safe.
///
/// Gaps between `MIN_SNAP_GAP` and this are still used, but only when no
/// cleaner gap is in range.
pub const CLEAN_SNAP_GAP: f64 = 0.40;

/// How far an edge may travel to reach a gap.
///
/// Bounded because the model chose these times to match what is on screen;
/// dragging an edge a long way to find silence would desynchronise narration
/// from the visuals it describes.
pub const SNAP_SEARCH_WINDOW: f64 = 1.0;

/// Inset from a gap's own edge, absorbing `silencedetect` boundary imprecision
/// so the snap lands solidly inside the quiet rather than on its lip.
pub const SNAP_PAD: f64 = 0.05;

/// Fraction of the video that must be silent before the source counts as
/// "effectively silent" and snapping is skipped entirely.
///
/// A fully silent screencast reports one enormous span covering the whole
/// timeline. Snapping into that is meaningless — every edge would "succeed" and
/// nothing would improve — so the whole pass is skipped, which is also the
/// honest no-op case for our most common input.
const EFFECTIVELY_SILENT_FRACTION: f64 = 0.95;

/// True when the source is silent enough that snapping cannot help.
pub fn is_effectively_silent(spans: &[SilenceSpan], video_duration: f64) -> bool {
    if video_duration <= 0.0 {
        return true;
    }
    let silent: f64 = spans.iter().map(SilenceSpan::duration).sum();
    silent / video_duration >= EFFECTIVELY_SILENT_FRACTION
}

/// Find a point inside a usable gap near `t`, or `None` to leave `t` alone.
///
/// Prefers gaps at least `CLEAN_SNAP_GAP` wide, and among equally clean
/// candidates the one requiring the smallest move.
fn snap_point(t: f64, spans: &[SilenceSpan]) -> Option<f64> {
    let mut best: Option<(bool, f64, f64)> = None; // (is_clean, distance, target)

    for span in spans {
        let width = span.duration();
        if width < MIN_SNAP_GAP {
            continue;
        }
        // Aim `SNAP_PAD` inside the gap, but never past its midpoint — on a
        // gap barely wider than 2×SNAP_PAD the padded points would cross.
        let target = if span.contains(t) {
            // Already inside a usable gap: nothing to gain by moving.
            return None;
        } else if t < span.start {
            (span.start + SNAP_PAD).min(span.midpoint())
        } else {
            (span.end - SNAP_PAD).max(span.midpoint())
        };

        let distance = (target - t).abs();
        if distance > SNAP_SEARCH_WINDOW {
            continue;
        }
        let is_clean = width >= CLEAN_SNAP_GAP;
        let better = match best {
            None => true,
            // A clean gap always beats a merely-usable one; otherwise closer wins.
            Some((best_clean, best_dist, _)) => match (is_clean, best_clean) {
                (true, false) => true,
                (false, true) => false,
                _ => distance < best_dist,
            },
        };
        if better {
            best = Some((is_clean, distance, target));
        }
    }

    best.map(|(_, _, target)| target)
}

/// Nudge every segment edge into a nearby silence gap.
///
/// Returns `segments` untouched when there is no usable silence map. Callers
/// must still run [`normalize_timeline`] afterwards: snapping can push an edge
/// past a neighbour or below the minimum segment length, and normalization is
/// what re-establishes those invariants.
pub fn snap_to_silence(
    mut segments: Vec<Segment>,
    spans: &[SilenceSpan],
    video_duration: f64,
) -> Vec<Segment> {
    if spans.is_empty() || is_effectively_silent(spans, video_duration) {
        return segments;
    }

    let mut moved = 0usize;
    for seg in &mut segments {
        if let Some(start) = snap_point(seg.start_seconds, spans) {
            // Never let a snap invert or collapse the segment.
            if start < seg.end_seconds - 0.3 {
                seg.start_seconds = start.max(0.0);
                moved += 1;
            }
        }
        if let Some(end) = snap_point(seg.end_seconds, spans) {
            if end > seg.start_seconds + 0.3 {
                seg.end_seconds = end.min(video_duration.max(0.0));
                moved += 1;
            }
        }
    }

    if moved > 0 {
        tracing::info!(
            "snapped {moved} segment edge(s) into silence gaps across {} spans",
            spans.len()
        );
    }
    segments
}

/// Render the usable silence gaps as a prompt block.
///
/// Gives the model the same information the snap pass uses, so it can place
/// segments well in the first place rather than being corrected afterwards.
/// Returns an empty string when there is nothing useful to say.
pub fn describe_silence_windows(spans: &[SilenceSpan], video_duration: f64) -> String {
    if spans.is_empty() || is_effectively_silent(spans, video_duration) {
        return String::new();
    }
    let usable: Vec<&SilenceSpan> = spans
        .iter()
        .filter(|s| s.duration() >= MIN_SNAP_GAP)
        .collect();
    if usable.is_empty() {
        return String::new();
    }

    // Cap the list: a chatty source can produce hundreds of gaps, and the
    // widest ones are the ones worth spending tokens on.
    const MAX_LISTED: usize = 40;
    let mut listed: Vec<&&SilenceSpan> = usable.iter().collect();
    listed.sort_by(|a, b| b.duration().total_cmp(&a.duration()));
    listed.truncate(MAX_LISTED);
    listed.sort_by(|a, b| a.start.total_cmp(&b.start));

    let windows: Vec<String> = listed
        .iter()
        .map(|s| format!("{:.2}-{:.2}", s.start, s.end))
        .collect();

    format!(
        "\n\n## EXISTING AUDIO IN THIS VIDEO\n\n\
         The source already has its own audio, so narration must not talk over \
         it. These are the quiet windows (seconds), widest {} of {} shown:\n\n\
         {}\n\n\
         Start and end each segment inside one of these windows wherever the \
         visuals allow. A segment that begins mid-sentence over existing speech \
         makes both tracks unintelligible.",
        listed.len(),
        usable.len(),
        windows.join(", ")
    )
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

/// True when `code` appears in `msg` as an isolated number — i.e. with no
/// digit on either side. Without this, an error string that happened to
/// contain "429" anywhere (e.g. "request 429 of 1000") would falsely
/// trigger the rate-limit retry path. Mirrors the frontend's
/// `hasHttpStatus` helper in `src/lib/errorMessages.ts`.
fn contains_status_code(msg: &str, code: u16) -> bool {
    let needle = code.to_string();
    let bytes = msg.as_bytes();
    let nlen = needle.len();
    if bytes.len() < nlen {
        return false;
    }
    let needle_bytes = needle.as_bytes();
    for i in 0..=bytes.len() - nlen {
        if &bytes[i..i + nlen] != needle_bytes {
            continue;
        }
        let before_ok = i == 0 || !bytes[i - 1].is_ascii_digit();
        let after_ok = i + nlen == bytes.len() || !bytes[i + nlen].is_ascii_digit();
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

/// Check if an error is a rate limit (429) or overloaded (529) that should
/// be retried. Matches `NarratorError::RateLimited` (the explicit signal),
/// status codes 429/529 as isolated numbers, and the textual variants
/// providers use.
fn is_rate_limit_error(err: &NarratorError) -> bool {
    // A billing/credit problem is permanent until the user acts — never retry
    // it, even if its detail text happens to mention a limit.
    if matches!(err, NarratorError::InsufficientCredit(_)) {
        return false;
    }
    if matches!(err, NarratorError::RateLimited) {
        return true;
    }
    let msg = err.to_string().to_lowercase();
    contains_status_code(&msg, 429)
        || contains_status_code(&msg, 529)
        || msg.contains("rate limit")
        || msg.contains("rate_limit")
        || msg.contains("too many requests")
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

/// Progress + cancellation context for `generate_with_retry`. When present, the
/// retry wrapper reports a live per-second countdown during rate-limit backoff
/// ("Rate limited — retrying in 42s") instead of leaving the bar frozen on the
/// caller's last label, and it checks the cancel flag each second so Cancel is
/// honored mid-wait rather than after the full 110s of backoff has elapsed. It
/// also drives the in-flight `· Ns elapsed` heartbeat, so callers pass this
/// INSTEAD of wrapping the call in `with_heartbeat` (doing both would let the
/// heartbeat clobber the countdown message every 1.5s).
struct RetryContext {
    on_progress: ProgressCallback,
    /// Narration-stage fraction (0..=1) to hold the bar at while retrying.
    fraction: f64,
    /// Label for the in-flight heartbeat ("Analyzing batch 1 of 2").
    label: String,
    cancel: Option<Arc<AtomicBool>>,
}

/// Call an AI provider with exponential backoff on rate limit errors.
///
/// `schema`, when present, pins the response shape at the API level instead of
/// asking for it in prose. Every JSON-returning path passes one; the paths that
/// legitimately want free text (a single rewritten segment) pass `None`.
async fn generate_with_retry(
    provider: &dyn AiProvider,
    system_prompt: &str,
    user_message: serde_json::Value,
    ctx: Option<&RetryContext>,
    schema: Option<&ResponseSchema>,
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
            match ctx {
                // Visible, cancellable countdown: emit one message per second so
                // the user sees the wait tick down and can Cancel it.
                Some(c) => {
                    for remaining in (1..=delay_secs).rev() {
                        if crate::cancel::is_cancelled(&c.cancel) {
                            return Err(NarratorError::Cancelled);
                        }
                        (c.on_progress)(
                            c.fraction,
                            Some(format!(
                                "Rate limited — retrying in {remaining}s (attempt {attempt} of {max_retries})"
                            )),
                        );
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                }
                None => tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await,
            }
        }
        // Run the call, driving the in-flight heartbeat when we have a context.
        // Both trait methods return the same boxed future type under
        // `async_trait`, so this dispatches without extra allocation.
        let call = match schema {
            Some(s) => provider.generate_with_schema(system_prompt, user_message.clone(), s),
            None => provider.generate(system_prompt, user_message.clone()),
        };
        let outcome = match ctx {
            Some(c) => {
                with_heartbeat(
                    &Some(c.on_progress.clone()),
                    c.fraction,
                    c.label.clone(),
                    call,
                )
                .await
            }
            None => call.await,
        };
        match outcome {
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

/// Share of the narration progress band consumed by the per-chunk API calls.
///
/// The remainder is reserved for the passes that run *after* the last chunk —
/// polish (up to 90 s), timeline stretch, short-segment merge. Without this
/// reservation the bar reached the top of the band and froze there, because the
/// frontend clamps progress monotonically forward and every later report was
/// numerically lower.
const CHUNK_SPAN: f64 = 0.85;

/// Fraction reported while the polish pass runs.
const POLISH_FRACTION: f64 = 0.90;

/// Fraction reported once polish is done and the final passes run.
const FINALIZE_FRACTION: f64 = 0.96;

// The post-chunk fractions must sit ABOVE the chunk ceiling. The frontend clamps
// progress monotonic-forward, so a lower value is silently discarded — which is
// precisely how the bar came to freeze at the top of the band during polish.
const _: () = assert!(POLISH_FRACTION > CHUNK_SPAN);
const _: () = assert!(FINALIZE_FRACTION > POLISH_FRACTION);

/// Generate a narration script.
///
/// `video_duration` is the **authoritative** length of the video, measured by
/// ffprobe. It is deliberately not derived from the model's
/// `total_duration_seconds`: on the chunked path the first chunk only ever sees
/// its own slice of frames, so it reports that slice's length. Trusting it made
/// `normalize_timeline` delete every segment past the first chunk — a 3:40 video
/// silently shipping 53 seconds of narration.
#[allow(clippy::too_many_arguments)]
pub async fn generate_narration(
    provider: &dyn AiProvider,
    system_prompt: &str,
    user_message: serde_json::Value,
    style: &str,
    language: &str,
    video_duration: f64,
    resume_segments: Vec<Segment>,
    on_segment: Option<SegmentCallback>,
    on_progress: Option<ProgressCallback>,
    cancel: Option<Arc<AtomicBool>>,
) -> Result<NarrationScript, NarratorError> {
    // First pass.
    let mut script = generate_narration_once(
        provider,
        system_prompt,
        user_message.clone(),
        video_duration,
        resume_segments.clone(),
        on_segment.clone(),
        on_progress.clone(),
        cancel.clone(),
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
            video_duration,
            Vec::new(),
            None,
            on_progress,
            cancel.clone(),
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
    video_duration: f64,
    resume_segments: Vec<Segment>,
    on_segment: Option<SegmentCallback>,
    on_progress: Option<ProgressCallback>,
    cancel: Option<Arc<AtomicBool>>,
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
            video_duration,
            resume_segments,
            on_segment.clone(),
            on_progress.clone(),
            cancel.clone(),
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
        // Same visible/cancellable backoff as the chunked path, anchored at the
        // 5% kickoff mark.
        let ctx = on_progress.clone().map(|cb| RetryContext {
            on_progress: cb,
            fraction: 0.05,
            label: "Generating narration with AI…".to_string(),
            cancel: cancel.clone(),
        });
        generate_with_retry(
            provider,
            system_prompt,
            user_message,
            ctx.as_ref(),
            Some(&response_schema::narration_script()),
        )
        .await?
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
    // The measured video length wins over whatever the model reported. On the
    // chunked path the model's figure describes one chunk's slice, and using it
    // here deleted every segment past that slice. Fall back to its value only
    // when we genuinely have no measurement.
    let duration = if video_duration > 0.0 {
        video_duration
    } else if script.total_duration_seconds > 0.0 {
        script.total_duration_seconds
    } else {
        script.segments.last().map(|s| s.end_seconds).unwrap_or(0.0) + 60.0
    };
    if (script.total_duration_seconds - duration).abs() > 1.0 {
        tracing::warn!(
            "model reported total_duration_seconds={:.1} but the video is {:.1}s — using the measured value",
            script.total_duration_seconds,
            duration
        );
    }
    // Pin the header to the truth so every downstream consumer (stretch
    // heuristics, export, the Review UI) agrees with the actual video.
    script.total_duration_seconds = duration;
    script.segments = normalize_timeline(std::mem::take(&mut script.segments), duration);

    // Merge sub-2.5s fragments into their neighbors. Humans can't comfortably
    // speak more than a few words in under 2.5 seconds, and TTS either
    // speeds up unnaturally or overruns when a slot is this short. Merging
    // upfront avoids the audio/video desync downstream.
    script.segments = merge_short_segments(std::mem::take(&mut script.segments), 2.5);

    // Keep the bar moving through the tail. These passes are fast, but polish
    // (below) is not, and a frozen bar reads as a hung app.
    if let Some(cb) = on_progress.as_ref() {
        cb(FINALIZE_FRACTION, Some("Finalizing timeline".to_string()));
    }

    // Chunked generation is prone to producing a choppy, fragmented script
    // because each chunk only sees a 10-frame window. A single polish pass
    // gives the AI the whole script at once to dedupe, merge, and smooth.
    // Best-effort: if the polish call fails, returns unparseable output, or
    // exceeds POLISH_TIMEOUT we keep the unpolished script rather than
    // breaking generation entirely.
    if was_chunked && script.segments.len() > 3 {
        // The polish path stacks two retry layers (provider-level for 429/529
        // and generate_with_retry's 5/15/30/60s backoff) on top of a 120s
        // HTTP timeout, so a single struggling call could pin the bar at 99%
        // for several minutes. Bound the whole pass — polish is best-effort,
        // not worth that wait.
        const POLISH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

        // Wrap the polish call in a heartbeat — it can take 10-30s on a long
        // script and is the silent tail right before the final "Complete"
        // label. Anchor at fraction 0.98 so the bar sits near the end while
        // ticking; real completion bumps it to 1.0 via the caller.
        let polish_label = "Polishing narration".to_string();
        let polish_with_deadline =
            tokio::time::timeout(POLISH_TIMEOUT, polish_script(provider, &script, 2.5));
        match with_heartbeat(
            &on_progress,
            POLISH_FRACTION,
            polish_label,
            polish_with_deadline,
        )
        .await
        {
            Ok(Ok(polished)) => {
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
            Ok(Err(e)) => {
                tracing::warn!("AI polish pass failed, keeping unpolished script: {e}");
            }
            Err(_elapsed) => {
                tracing::warn!(
                    "AI polish pass exceeded {}s, keeping unpolished script",
                    POLISH_TIMEOUT.as_secs()
                );
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

    let response_text = generate_with_retry(
        provider,
        &system_prompt,
        user_message,
        None,
        Some(&response_schema::narration_script()),
    )
    .await?;
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

    let response_text = generate_with_retry(
        provider,
        &system_prompt,
        user_message,
        None,
        Some(&response_schema::narration_script()),
    )
    .await?;

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

/// Upper bound on frames sent per critique call. Large enough that each
/// segment has a nearby visual anchor on a typical 30–50 segment script;
/// small enough to keep the multimodal prompt cheap.
const CRITIQUE_MAX_SAMPLES: usize = 10;

/// A sampled frame paired with the segment indices it serves as visual
/// ground truth for. The critique pass uses this to tell the model which
/// segments it should audit against each frame, so the model doesn't
/// speculate about segments that have no nearby frame.
#[derive(Debug, Clone)]
struct FrameScope {
    frame: Frame,
    segment_indices: Vec<usize>,
}

/// Base64-encoded frame + the segment indices it scopes. Cached across
/// iterations so iteration 2 doesn't re-read and re-encode the same JPEGs.
#[derive(Clone)]
struct EncodedScope {
    timestamp_seconds: f64,
    segment_indices: Vec<usize>,
    b64: String,
}

/// Run up to 2 critique+refine iterations on `script`. Each iteration asks
/// the model whether flagged segments' narration matches what's visible in
/// the nearest sampled frame; any flagged segment is rewritten via
/// `refine_segment` using the critique's own suggestion as the instruction.
///
/// Returns the (possibly updated) script. Never fails the whole pipeline —
/// any critique-side error downgrades to "skip critique, return as-is".
///
/// Bounded cost: one multimodal critique call per iteration (with up to
/// CRITIQUE_MAX_SAMPLES frames) + one text-only `refine_segment` call per
/// mismatch per iteration (capped at MAX_REFINES_PER_ITER). Default-off,
/// gated by `GenerationParams::strict_mode`.
///
/// Segments rewritten in iteration 1 are excluded from iteration 2 so the
/// loop can't oscillate or compound conflicting edits on the same segment.
pub async fn self_critique_and_refine(
    provider: &dyn AiProvider,
    script: NarrationScript,
    frames: &[Frame],
    on_segment: Option<SegmentCallback>,
    on_progress: Option<ProgressCallback>,
    cancel_flag: Option<Arc<AtomicBool>>,
) -> NarrationScript {
    const MAX_ITERATIONS: usize = 2;
    const MAX_REFINES_PER_ITER: usize = 5;

    let mut script = script;
    let scopes = pick_frames_with_segment_map(frames, &script.segments, CRITIQUE_MAX_SAMPLES);
    if scopes.is_empty() {
        tracing::info!("self-critique skipped: no frames available");
        return script;
    }

    // Encode each sampled frame once — iterations re-use the same bytes.
    // Missing-frame entries are silently dropped here (not a fatal error).
    let mut encoded: Vec<EncodedScope> = Vec::with_capacity(scopes.len());
    for scope in scopes {
        if !scope.frame.path.exists() {
            continue;
        }
        match video_engine::frame_to_base64(&scope.frame.path) {
            Ok(b64) => encoded.push(EncodedScope {
                timestamp_seconds: scope.frame.timestamp_seconds,
                segment_indices: scope.segment_indices,
                b64,
            }),
            Err(e) => tracing::warn!(
                "self-critique: skipping frame {} ({})",
                scope.frame.path.display(),
                e
            ),
        }
    }
    if encoded.is_empty() {
        tracing::info!("self-critique skipped: no encodable frames");
        return script;
    }

    // Segments rewritten in a previous iteration are skipped thereafter —
    // prevents oscillation and stops the model from compounding conflicting
    // edits on the same segment across iterations.
    let mut rewritten: std::collections::HashSet<usize> = std::collections::HashSet::new();

    for iter in 0..MAX_ITERATIONS {
        if cancelled(&cancel_flag) {
            tracing::info!("self-critique cancelled before iteration {}", iter + 1);
            return script;
        }
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
        let mismatches = match run_critique(provider, &script, &encoded).await {
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

        // Dedup by segment_index (a model that lists two issues for one
        // segment would otherwise trigger two sequential rewrites that
        // compound on each other). Also drop segments already refined in a
        // prior iteration.
        let mut seen = std::collections::HashSet::new();
        let deduped: Vec<Mismatch> = mismatches
            .into_iter()
            .filter(|m| !rewritten.contains(&m.segment_index))
            .filter(|m| seen.insert(m.segment_index))
            .collect();

        tracing::info!(
            "self-critique iteration {}: {} unique mismatches (capped to {})",
            iter + 1,
            deduped.len(),
            MAX_REFINES_PER_ITER
        );

        for mismatch in deduped.into_iter().take(MAX_REFINES_PER_ITER) {
            if cancelled(&cancel_flag) {
                tracing::info!("self-critique cancelled mid-iteration");
                return script;
            }
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
                    rewritten.insert(mismatch.segment_index);
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

/// Read an optional cancel flag.
fn cancelled(flag: &Option<Arc<AtomicBool>>) -> bool {
    flag.as_ref()
        .map(|f| f.load(Ordering::SeqCst))
        .unwrap_or(false)
}

/// Pick up to `max_samples` frames evenly distributed across the timeline,
/// and for each, identify which segments it should audit. Every segment is
/// assigned to exactly one frame — the frame whose timestamp is nearest the
/// segment's midpoint — so the critique prompt can scope the model's audit
/// to segments with real visual ground truth.
///
/// Frames that end up scoping zero segments are dropped (nothing to audit).
fn pick_frames_with_segment_map(
    frames: &[Frame],
    segments: &[Segment],
    max_samples: usize,
) -> Vec<FrameScope> {
    if frames.is_empty() || segments.is_empty() || max_samples == 0 {
        return Vec::new();
    }
    let n = frames.len();
    let count = max_samples.min(n);
    // Evenly-spaced picks: bin edges at i/count, frame at bin midpoint.
    let mut seen = std::collections::HashSet::new();
    let mut picked: Vec<Frame> = Vec::with_capacity(count);
    for i in 0..count {
        let pos = ((i as f64 + 0.5) * (n as f64 / count as f64)).floor() as usize;
        let idx = pos.min(n - 1);
        if seen.insert(idx) {
            picked.push(frames[idx].clone());
        }
    }

    // Assign each segment to its nearest picked frame.
    let mut scopes: Vec<Vec<usize>> = vec![Vec::new(); picked.len()];
    for seg in segments {
        let mid = 0.5 * (seg.start_seconds + seg.end_seconds);
        let best = picked
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                (a.timestamp_seconds - mid)
                    .abs()
                    .partial_cmp(&(b.timestamp_seconds - mid).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| i);
        if let Some(i) = best {
            scopes[i].push(seg.index);
        }
    }

    picked
        .into_iter()
        .zip(scopes)
        .filter(|(_, s)| !s.is_empty())
        .map(|(frame, segment_indices)| FrameScope {
            frame,
            segment_indices,
        })
        .collect()
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
    encoded: &[EncodedScope],
) -> Result<Vec<Mismatch>, NarratorError> {
    // The prompt explicitly scopes each frame to a specific segment range —
    // the model must only flag mismatches for segments it has real visual
    // ground truth for, rather than speculating across the whole timeline
    // based on a sample that covers only part of it.
    let system_prompt = "You are reviewing a narration script against sampled video frames. \
        Each frame below is scoped to a specific list of segment indices — audit ONLY those \
        segments against the matching frame. Do NOT flag segments that are not scoped to any \
        frame; you cannot see them. For segments you do audit, flag only real disagreements \
        with the visible content (wrong subject, wrong UI element, contradicted action) — \
        ignore minor wording. If the scoped segments look correct, return an empty array. \
        Respond with ONLY a JSON object of the form \
        {\"mismatches\": [{\"segment_index\": <int>, \"reason\": \"<why>\", \"suggestion\": \"<concrete rewrite guidance>\"}]} \
        — no markdown, no commentary.";

    // Compact the script into a single plain-text listing — the model gets
    // all segments for context but is instructed not to flag the unscoped
    // ones.
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
        "text": format!(
            "Narration script (index, window, text):\n{script_text}\n\n\
             {n} sampled frames follow. Each frame lists the segment indices \
             it is meant to audit — restrict your mismatches to those indices.",
            n = encoded.len()
        ),
    }));
    for scope in encoded {
        let indices = scope
            .segment_indices
            .iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        content.push(json!({
            "type": "text",
            "text": format!(
                "[Frame at {:.1}s — audit segments: {}]",
                scope.timestamp_seconds, indices
            ),
        }));
        content.push(json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": "image/jpeg",
                "data": scope.b64,
            }
        }));
    }

    let user_message = serde_json::Value::Array(content);
    // Goes through the same retry layer as every other AI call. Without this,
    // critique would skip the rate-limit backoff and surface a transient 429
    // as "self-critique skipped".
    let response = generate_with_retry(
        provider,
        system_prompt,
        user_message,
        None,
        Some(&response_schema::critique()),
    )
    .await?;
    parse_critique_response(&response)
}

/// Extract the first balanced `{...}` object from `s`, tracking brace depth
/// and string literals so prose containing stray `{` or `}` characters
/// (e.g. `"Here's the JSON {output}:"`) doesn't throw off the slice.
fn extract_first_json_object(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    let start = s.find('{')?;
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escape = false;
    for (i, &b) in bytes[start..].iter().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        if in_string {
            match b {
                b'\\' => escape = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[start..start + i + 1]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Parse a critique JSON response tolerant of code fences / stray text.
fn parse_critique_response(raw: &str) -> Result<Vec<Mismatch>, NarratorError> {
    let trimmed = raw
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    // Use balanced-brace extraction so prose with stray `{` (e.g.
    // "Here {'s} the JSON:") doesn't cause a slice that includes prefix
    // text and fails to parse.
    let slice = extract_first_json_object(trimmed).unwrap_or(trimmed);
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

/// Group a finished script's segments into named chapters.
///
/// Runs after generation rather than as part of it: asking one call to write
/// narration *and* structure it produces a worse version of both, the same
/// reason the frame-selection survey is its own pass.
///
/// The model returns segment indices, not timestamps — it already has the
/// segment list, and letting it invent seconds invites values that drift off
/// the real boundaries. Timestamps are looked up here from the segment each
/// chapter names, so a chapter can never start somewhere no segment does.
pub async fn generate_chapters(
    provider: &dyn AiProvider,
    script: &NarrationScript,
) -> Result<Vec<Chapter>, NarratorError> {
    // Nothing to divide. Cheaper to answer here than to pay for a round trip
    // that will come back empty anyway.
    if script.segments.len() < 3 {
        return Ok(Vec::new());
    }

    let system_prompt = "You divide a narration script into chapters, the way a         product demo is broken into sections a viewer can jump between. Group         CONSECUTIVE segments; every chapter starts where the previous one ends.         Aim for 3-8 chapters for a typical video and never more than one per         three segments — chapters are navigation, not an outline of every         sentence. Title each by what the viewer sees happening in it, in 2-6         words, with no numbering and no trailing period. The first chapter must         start at segment 0. Return an empty list if the video is too short or         too uniform to divide meaningfully.";

    let mut listing = String::with_capacity(script.segments.len() * 96);
    for (i, seg) in script.segments.iter().enumerate() {
        listing.push_str(&format!(
            "[{i}] {:.1}s-{:.1}s: {}\n",
            seg.start_seconds,
            seg.end_seconds,
            seg.text.trim()
        ));
    }

    let user_message = json!(format!(
        "Video title: {}\nTotal duration: {:.0}s\n\nSegments:\n{listing}",
        script.title, script.total_duration_seconds
    ));

    let response = generate_with_retry(
        provider,
        system_prompt,
        user_message,
        None,
        Some(&response_schema::chapters()),
    )
    .await?;

    Ok(parse_chapters_response(&response, script))
}

/// Turn the model's `{title, start_segment}` list into chapters anchored to
/// real segment boundaries.
///
/// Defensive on purpose — a chapter list is cosmetic, so every malformed entry
/// is dropped rather than failing the whole call and losing the good ones.
/// Out-of-range indices, duplicate starts, blank titles and out-of-order
/// entries are all things models produce occasionally.
fn parse_chapters_response(raw: &str, script: &NarrationScript) -> Vec<Chapter> {
    let trimmed = raw
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let slice = extract_first_json_object(trimmed).unwrap_or(trimmed);
    let Ok(value) = serde_json::from_str::<serde_json::Value>(slice) else {
        tracing::warn!("chapter JSON parse failed; continuing without chapters");
        return Vec::new();
    };
    let Some(arr) = value.get("chapters").and_then(|v| v.as_array()) else {
        return Vec::new();
    };

    let mut out: Vec<Chapter> = Vec::with_capacity(arr.len());
    let mut seen: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for item in arr {
        let title = item
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if title.is_empty() {
            continue;
        }
        let Some(idx) = item
            .get("start_segment")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
        else {
            continue;
        };
        let Some(seg) = script.segments.get(idx) else {
            continue; // hallucinated index past the end of the script
        };
        if !seen.insert(idx) {
            continue; // two chapters cannot start on the same segment
        }
        out.push(Chapter {
            title,
            start_seconds: seg.start_seconds,
            start_segment: idx,
        });
    }

    out.sort_by_key(|c| c.start_segment);
    // A list that does not open at the first segment leaves the opening
    // narration in no chapter at all; anchor it rather than drop it.
    if let Some(first) = out.first_mut() {
        if first.start_segment != 0 {
            first.start_segment = 0;
            first.start_seconds = script.segments[0].start_seconds;
        }
    }
    out
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

    // No schema: this path returns a bare rewritten sentence, not an object.
    let response = generate_with_retry(provider, system_prompt, user_message, None, None).await?;

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

    let response_text = generate_with_retry(
        provider,
        &system_prompt,
        user_message,
        None,
        Some(&response_schema::narration_script()),
    )
    .await?;
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
mod request_shape_tests {
    use super::*;

    fn user_msg() -> serde_json::Value {
        json!([{ "type": "text", "text": "hi" }])
    }

    // ── Anthropic ────────────────────────────────────────────────────────────

    /// Opus 4.7+ removed `temperature`/`top_p`/`top_k` — sending any of them is a
    /// hard 400, so the builder must omit the key entirely (not clamp it).
    #[test]
    fn claude_omits_temperature_on_models_that_reject_it() {
        for model in [
            "claude-opus-5",
            "claude-sonnet-5",
            "claude-fable-5",
            "claude-opus-4-8",
            "claude-opus-4-7",
        ] {
            let body = build_claude_body(model, 0.7, ReasoningEffort::Balanced, "sys", user_msg());
            assert!(
                body.get("temperature").is_none(),
                "{model} must not receive `temperature` (400)"
            );
        }
    }

    /// Models that still accept sampling params should keep getting them, so we
    /// don't silently change behaviour for the older tiers.
    #[test]
    fn claude_keeps_temperature_on_models_that_accept_it() {
        for model in ["claude-haiku-4-5", "claude-sonnet-4-6", "claude-opus-4-6"] {
            let body = build_claude_body(model, 0.42, ReasoningEffort::Balanced, "sys", user_msg());
            assert_eq!(
                body["temperature"].as_f64().unwrap(),
                0.42_f32 as f64,
                "{model} should still receive `temperature`"
            );
        }
    }

    /// Thinking must be the adaptive form and never `disabled`: `disabled` is a
    /// 400 on Fable 5, is rejected above `high` effort on Opus 5, and with
    /// thinking off these models can leak `<thinking>` tags into the visible
    /// response — which would break this app's strict JSON parse.
    #[test]
    fn claude_uses_adaptive_thinking_never_disabled() {
        for effort in [
            ReasoningEffort::Fast,
            ReasoningEffort::Balanced,
            ReasoningEffort::Thorough,
            ReasoningEffort::Max,
        ] {
            let body = build_claude_body("claude-opus-5", 0.7, effort, "sys", user_msg());
            assert_eq!(body["thinking"]["type"], "adaptive");
            assert!(body["thinking"].get("budget_tokens").is_none());
        }
    }

    #[test]
    fn claude_maps_effort_levels() {
        let cases = [
            (ReasoningEffort::Fast, "low"),
            (ReasoningEffort::Balanced, "medium"),
            (ReasoningEffort::Thorough, "high"),
            (ReasoningEffort::Max, "max"),
        ];
        for (effort, expected) in cases {
            let body = build_claude_body("claude-sonnet-5", 0.7, effort, "sys", user_msg());
            assert_eq!(body["output_config"]["effort"], expected);
        }
    }

    /// `effort` errors on Sonnet 4.5 / Haiku 4.5, so it must not be sent there.
    #[test]
    fn claude_omits_effort_on_models_without_support() {
        let body = build_claude_body(
            "claude-haiku-4-5",
            0.7,
            ReasoningEffort::Max,
            "sys",
            user_msg(),
        );
        assert!(body.get("output_config").is_none());
        assert!(body.get("thinking").is_none());
    }

    /// Thinking tokens and response text share `max_tokens`; a thinking model
    /// sized at the old 8192 can truncate the JSON payload mid-object.
    #[test]
    fn claude_gives_thinking_models_more_output_headroom() {
        let thinking = build_claude_body(
            "claude-opus-5",
            0.7,
            ReasoningEffort::Balanced,
            "s",
            user_msg(),
        );
        let plain = build_claude_body(
            "claude-haiku-4-5",
            0.7,
            ReasoningEffort::Balanced,
            "s",
            user_msg(),
        );
        assert!(thinking["max_tokens"].as_u64().unwrap() > plain["max_tokens"].as_u64().unwrap());
    }

    // ── OpenAI ───────────────────────────────────────────────────────────────

    /// The GPT-5.6 variants must be recognised as reasoning models, or they get
    /// `max_tokens` + `temperature` and 400.
    #[test]
    fn openai_treats_gpt56_variants_as_reasoning_models() {
        for model in ["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna", "gpt-5.6"] {
            assert!(
                is_openai_reasoning_model(model),
                "{model} must be treated as a reasoning model"
            );
            let body = build_openai_body(model, 0.7, ReasoningEffort::Thorough, "sys", user_msg());
            assert!(body.get("temperature").is_none(), "{model}: no temperature");
            assert!(body.get("max_tokens").is_none(), "{model}: no max_tokens");
            assert!(body["max_completion_tokens"].is_number());
        }
    }

    /// Chat Completions takes the *flat* `reasoning_effort`. The nested
    /// `reasoning: {effort}` object is the Responses API shape and 400s here.
    #[test]
    fn openai_uses_flat_reasoning_effort_for_chat_completions() {
        let body = build_openai_body(
            "gpt-5.6-sol",
            0.7,
            ReasoningEffort::Balanced,
            "sys",
            user_msg(),
        );
        assert_eq!(body["reasoning_effort"], "medium");
        assert!(
            body.get("reasoning").is_none(),
            "must not send the Responses-API nested object"
        );
    }

    #[test]
    fn openai_non_reasoning_model_keeps_temperature() {
        let body = build_openai_body("gpt-4o", 0.33, ReasoningEffort::Max, "sys", user_msg());
        assert!((body["temperature"].as_f64().unwrap() - 0.33_f32 as f64).abs() < 1e-9);
        assert!(body.get("reasoning_effort").is_none());
        assert!(body["max_tokens"].is_number());
    }

    // ── Gemini ───────────────────────────────────────────────────────────────

    #[test]
    fn gemini_sends_thinking_level_on_gemini_3() {
        let body = build_gemini_body(
            "gemini-3.6-flash",
            0.7,
            ReasoningEffort::Thorough,
            "sys",
            vec![json!({"text": "hi"})],
        );
        assert_eq!(body["generationConfig"]["thinkingLevel"], "high");
        // Legacy 2.5 knob is mutually exclusive with thinkingLevel — never both.
        assert!(body["generationConfig"].get("thinkingBudget").is_none());
    }

    /// Gemini's ladder stops at `high`; `max` must clamp rather than send a
    /// value the API would reject.
    #[test]
    fn gemini_clamps_max_to_high() {
        let body = build_gemini_body(
            "gemini-3.6-flash",
            0.7,
            ReasoningEffort::Max,
            "sys",
            vec![json!({"text": "hi"})],
        );
        assert_eq!(body["generationConfig"]["thinkingLevel"], "high");
    }

    /// Gemini 2.5 uses the older `thinkingBudget`, so the new key must not be
    /// sent to it.
    #[test]
    fn gemini_omits_thinking_level_on_gemini_25() {
        let body = build_gemini_body(
            "gemini-2.5-flash",
            0.7,
            ReasoningEffort::Max,
            "sys",
            vec![json!({"text": "hi"})],
        );
        assert!(body["generationConfig"].get("thinkingLevel").is_none());
    }

    /// Strict JSON output is what keeps Gemini from emitting Python-dict-style
    /// responses that fail the downstream parse — must survive the refactor.
    #[test]
    fn gemini_keeps_strict_json_response_type() {
        let body = build_gemini_body(
            "gemini-3.6-flash",
            0.7,
            ReasoningEffort::Balanced,
            "sys",
            vec![json!({"text": "hi"})],
        );
        assert_eq!(
            body["generationConfig"]["responseMimeType"],
            "application/json"
        );
        assert_eq!(body["systemInstruction"]["parts"][0]["text"], "sys");
    }

    // ── Config plumbing ──────────────────────────────────────────────────────

    /// Projects saved before reasoning selection existed must still load.
    #[test]
    fn ai_config_defaults_reasoning_effort_when_absent() {
        let cfg: AiConfig = serde_json::from_str(
            r#"{"provider":"claude","model":"claude-sonnet-5","temperature":0.7}"#,
        )
        .expect("legacy AiConfig without reasoning_effort must deserialize");
        assert_eq!(cfg.reasoning_effort, ReasoningEffort::Balanced);
    }

    #[test]
    fn reasoning_effort_round_trips_as_lowercase() {
        let json = serde_json::to_string(&ReasoningEffort::Thorough).unwrap();
        assert_eq!(json, "\"thorough\"");
        assert_eq!(
            serde_json::from_str::<ReasoningEffort>("\"max\"").unwrap(),
            ReasoningEffort::Max
        );
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
    fn extract_first_json_object_tracks_nested_depth() {
        // The naive first-{ / last-} slice handles this too, but only
        // because last-} happens to match. Balanced tracking is what makes
        // it safe for nested-then-trailing-prose cases.
        let raw = r#"{"mismatches":[{"segment_index":0,"suggestion":"fix"}]}"#;
        let slice = extract_first_json_object(raw).unwrap();
        assert_eq!(slice, raw);
    }

    #[test]
    fn extract_first_json_object_ignores_braces_inside_strings() {
        // Critical regression target: a suggestion string containing `{` or
        // `}` inside a quoted value must not trick the brace counter into
        // closing early.
        let raw = r#"{"mismatches":[{"segment_index":0,"suggestion":"use the {foo} widget"}]}"#;
        let slice = extract_first_json_object(raw).unwrap();
        assert_eq!(slice, raw);
        // And the full parse works on this slice.
        let v: serde_json::Value = serde_json::from_str(slice).unwrap();
        assert_eq!(v["mismatches"][0]["segment_index"], 0);
    }

    #[test]
    fn extract_first_json_object_stops_at_balanced_close_before_trailing_prose() {
        let raw = "some prefix {\"mismatches\":[]} and then garbage text }}}";
        let slice = extract_first_json_object(raw).unwrap();
        assert_eq!(slice, "{\"mismatches\":[]}");
    }

    #[test]
    fn pick_frames_with_segment_map_assigns_each_segment_to_nearest_frame() {
        let mk_frame = |i: usize, t: f64| Frame {
            index: i,
            timestamp_seconds: t,
            path: std::path::PathBuf::from("/dev/null"),
            width: 0,
            height: 0,
        };
        let mk_seg = |i: usize, s: f64, e: f64| Segment {
            index: i,
            start_seconds: s,
            end_seconds: e,
            text: String::new(),
            visual_description: String::new(),
            emphasis: Vec::new(),
            pace: Pace::default(),
            pause_after_ms: 0,
            frame_refs: Vec::new(),
            voice_override: None,
        };
        // 20 frames spaced 1s apart, 8 segments spaced ~2.5s apart.
        let frames: Vec<Frame> = (0..20).map(|i| mk_frame(i, i as f64)).collect();
        let segments: Vec<Segment> = (0..8)
            .map(|i| mk_seg(i, i as f64 * 2.5, i as f64 * 2.5 + 2.0))
            .collect();

        let scopes = pick_frames_with_segment_map(&frames, &segments, 4);
        // At most `max_samples` sampled frames.
        assert!(scopes.len() <= 4);
        // Every segment ends up in exactly one scope.
        let total_assigned: usize = scopes.iter().map(|s| s.segment_indices.len()).sum();
        assert_eq!(total_assigned, segments.len());
    }

    #[test]
    fn pick_frames_with_segment_map_returns_empty_for_empty_inputs() {
        assert!(pick_frames_with_segment_map(&[], &[], 10).is_empty());
    }

    #[test]
    fn cancelled_respects_flag_state() {
        assert!(!cancelled(&None));
        let f = Arc::new(AtomicBool::new(false));
        assert!(!cancelled(&Some(f.clone())));
        f.store(true, Ordering::SeqCst);
        assert!(cancelled(&Some(f)));
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
    fn system_prompt_separates_hard_rules_from_craft_and_anti_patterns() {
        let prompt = build_system_prompt(&test_style(), &[], "", "en");
        // Three distinct blocks, in escalating-latitude order: what breaks the
        // export, what is taste, what to avoid.
        let hard = prompt.find("## HARD RULES").expect("hard rules block");
        let craft = prompt.find("## CRAFT").expect("craft block");
        let avoid = prompt.find("## AVOID").expect("avoid block");
        assert!(hard < craft && craft < avoid, "blocks out of order");

        // Hard rules must read as non-negotiable, craft explicitly as judgement.
        assert!(prompt[hard..craft].contains("MUST"));
        assert!(
            prompt[craft..avoid].contains("your judgement")
                || prompt[craft..avoid].contains("not rules"),
            "craft block must be marked optional"
        );
    }

    #[test]
    fn hard_rules_keep_every_export_breaking_constraint() {
        // These four are the ones that corrupt the export or the TTS output if
        // dropped. Asserted individually so a future prompt edit can't quietly
        // lose one while the section still looks plausible.
        let prompt = build_system_prompt(&test_style(), &[], "", "en");
        let hard = &prompt[prompt.find("## HARD RULES").unwrap()..prompt.find("## CRAFT").unwrap()];
        assert!(hard.contains("ONLY the JSON"), "JSON-only rule missing");
        assert!(hard.contains("total_duration_seconds"), "coverage missing");
        assert!(hard.contains("[pause]"), "speakable-text rule missing");
        assert!(hard.contains("WORD BUDGET"), "word budget missing");
        // The formula itself, not just the heading — it is what keeps TTS in sync.
        assert!(hard.contains("round((end_seconds - start_seconds)"));
    }

    #[test]
    fn anti_patterns_name_the_failure_modes_observed_in_output() {
        let prompt = build_system_prompt(&test_style(), &[], "", "en");
        let avoid = &prompt[prompt.find("## AVOID").unwrap()..];
        for pattern in [
            "verbatim",      // reading on-screen text aloud
            "In this video", // filler opener
            "seamlessly",    // empty intensifier
            "chrome",        // narrating the UI furniture
            "budget",        // cramming past the limit
        ] {
            assert!(
                avoid.contains(pattern),
                "anti-pattern list lost {pattern:?}"
            );
        }
    }

    fn script_with(n: usize) -> NarrationScript {
        let segments = (0..n)
            .map(|i| Segment {
                index: i,
                start_seconds: i as f64 * 10.0,
                end_seconds: (i as f64 + 1.0) * 10.0,
                text: format!("Segment {i}"),
                visual_description: String::new(),
                emphasis: vec![],
                pace: Pace::default(),
                pause_after_ms: 0,
                frame_refs: vec![],
                voice_override: None,
            })
            .collect();
        NarrationScript {
            title: "Demo".into(),
            total_duration_seconds: n as f64 * 10.0,
            segments,
            metadata: ScriptMetadata::default(),
            speech_rate_report: None,
            chapters: None,
        }
    }

    #[test]
    fn chapters_are_anchored_to_real_segment_timestamps() {
        let script = script_with(6);
        let raw = r#"{"chapters":[{"title":"Intro","start_segment":0},
                                  {"title":"Setup","start_segment":3}]}"#;
        let got = parse_chapters_response(raw, &script);
        assert_eq!(got.len(), 2);
        // 30.0 comes from the segment, not from anything the model said.
        assert_eq!(got[1].start_seconds, 30.0);
        assert_eq!(got[1].start_segment, 3);
    }

    #[test]
    fn chapters_drop_indices_past_the_end_of_the_script() {
        let script = script_with(4);
        let raw = r#"{"chapters":[{"title":"Intro","start_segment":0},
                                  {"title":"Ghost","start_segment":99}]}"#;
        let got = parse_chapters_response(raw, &script);
        assert_eq!(got.len(), 1, "hallucinated index must not survive");
        assert_eq!(got[0].title, "Intro");
    }

    #[test]
    fn chapters_drop_duplicate_starts_and_blank_titles() {
        let script = script_with(5);
        let raw = r#"{"chapters":[{"title":"Intro","start_segment":0},
                                  {"title":"Again","start_segment":0},
                                  {"title":"   ","start_segment":2}]}"#;
        let got = parse_chapters_response(raw, &script);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].title, "Intro");
    }

    #[test]
    fn chapters_are_sorted_and_anchored_to_the_opening_segment() {
        let script = script_with(6);
        // Out of order, and nothing starts at segment 0.
        let raw = r#"{"chapters":[{"title":"Later","start_segment":4},
                                  {"title":"Middle","start_segment":2}]}"#;
        let got = parse_chapters_response(raw, &script);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].title, "Middle", "must be sorted by segment");
        // The opening narration must belong to some chapter.
        assert_eq!(got[0].start_segment, 0);
        assert_eq!(got[0].start_seconds, 0.0);
    }

    #[test]
    fn chapters_survive_a_fenced_or_unparseable_response() {
        let script = script_with(4);
        let fenced = "```json\n{\"chapters\":[{\"title\":\"Intro\",\"start_segment\":0}]}\n```";
        assert_eq!(parse_chapters_response(fenced, &script).len(), 1);
        // Garbage is a dropped chapter list, never a failed export.
        assert!(parse_chapters_response("not json at all", &script).is_empty());
    }

    #[test]
    fn word_budget_rate_still_tracks_the_language() {
        // The budget is per-language; an English rate leaking into a Japanese
        // prompt would silently license 2.5x too much text.
        let en = build_system_prompt(&test_style(), &[], "", "en");
        let ja = build_system_prompt(&test_style(), &[], "", "ja");
        assert!(en.contains("150") && en.contains("words"));
        assert!(ja.contains("400") && ja.contains("characters"));
    }

    fn test_style() -> NarrationStyle {
        NarrationStyle {
            id: "technical".into(),
            label: "Technical".into(),
            description: "Technical deep-dive".into(),
            system_prompt: "You are narrating a technical video.".into(),
            pacing: "medium".into(),
            pause_markers: true,
        }
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
            reasoning_effort: ReasoningEffort::Balanced,
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
            reasoning_effort: ReasoningEffort::Balanced,
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
        let result = build_user_message(
            &[],
            "Test Video",
            "A test description",
            &metadata,
            "en",
            false,
        );
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

    // Always reports a rate limit, to exercise the retry backoff path.
    struct AlwaysRateLimited;

    #[async_trait]
    impl AiProvider for AlwaysRateLimited {
        async fn generate(
            &self,
            _system_prompt: &str,
            _user_message: serde_json::Value,
        ) -> Result<String, NarratorError> {
            Err(NarratorError::RateLimited)
        }
        fn name(&self) -> &str {
            "always-rate-limited"
        }
        fn model(&self) -> &str {
            "mock-v1"
        }
    }

    #[tokio::test]
    async fn test_retry_backoff_emits_countdown_and_honors_cancel() {
        use std::sync::Mutex;
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_from_cb = cancel.clone();
        let messages: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let messages_cb = messages.clone();
        // Flip the cancel flag as soon as the first countdown message lands.
        // The loop checks the flag at the top of each 1s tick, so this proves
        // the wait is interruptible mid-backoff instead of blocking the full 5s
        // (and that a visible message is emitted before the sleep).
        let cb: ProgressCallback = Arc::new(move |_frac, msg| {
            if let Some(m) = msg {
                messages_cb.lock().unwrap().push(m);
                cancel_from_cb.store(true, Ordering::SeqCst);
            }
        });
        let ctx = RetryContext {
            on_progress: cb,
            fraction: 0.3,
            label: "Analyzing batch 1 of 2".to_string(),
            cancel: Some(cancel),
        };

        let result =
            generate_with_retry(&AlwaysRateLimited, "sys", json!("msg"), Some(&ctx), None).await;

        // First call rate-limits → backoff; cancel set during the countdown
        // surfaces Cancelled rather than exhausting all four retries.
        assert!(matches!(result, Err(NarratorError::Cancelled)));
        let msgs = messages.lock().unwrap();
        assert!(
            msgs.iter().any(|m| m.contains("Rate limited — retrying")),
            "expected a visible rate-limit countdown message, got: {msgs:?}"
        );
    }

    #[test]
    fn test_billing_vs_rate_limit_classification() {
        // OpenAI surfaces no-credit as a 429 with insufficient_quota.
        let openai_quota = r#"{"error":{"message":"You exceeded your current quota, please check your plan and billing details.","type":"insufficient_quota"}}"#;
        assert!(matches!(
            classify_rate_or_billing(429, openai_quota, "OpenAI"),
            NarratorError::InsufficientCredit(_)
        ));

        // Anthropic returns low-credit as a 400 invalid_request_error, NOT a
        // 402/429 — classification must key off the body, not the status.
        let anthropic_credit = r#"{"error":{"type":"invalid_request_error","message":"Your credit balance is too low to access the Anthropic API. Please go to Plans & Billing to upgrade or purchase credits."}}"#;
        assert!(matches!(
            classify_rate_or_billing(400, anthropic_credit, "Claude"),
            NarratorError::InsufficientCredit(_)
        ));
        assert!(looks_like_billing_error(anthropic_credit));

        // A 402 is always billing, even with an empty/opaque body.
        assert!(matches!(
            classify_rate_or_billing(402, "", "Claude"),
            NarratorError::InsufficientCredit(_)
        ));

        // A genuine rate limit stays retryable.
        let real_rate_limit = r#"{"error":{"type":"rate_limit_error","message":"Number of request tokens has exceeded your per-minute rate limit."}}"#;
        let classified = classify_rate_or_billing(429, real_rate_limit, "Claude");
        assert!(matches!(classified, NarratorError::RateLimited));

        // Billing errors must never enter the retry backoff; rate limits must.
        assert!(!is_rate_limit_error(&NarratorError::InsufficientCredit(
            "out of credit".into()
        )));
        assert!(is_rate_limit_error(&NarratorError::RateLimited));

        // Cross-language contract: the serialized error (error.rs serializes via
        // Display) MUST carry this exact prefix — the frontend's toUserMessage /
        // isBillingError key off "credit or billing problem". Don't change one
        // side without the other.
        assert!(NarratorError::InsufficientCredit("x".into())
            .to_string()
            .starts_with("API credit or billing problem:"));
    }

    /// Exercises the exact status→variant wiring each provider now shares,
    /// using the real error bodies the three vendors return. This is the seam
    /// that misclassified a no-credit account as a rate limit.
    #[test]
    fn test_classify_error_response_wiring() {
        // OpenAI: no-credit is a 429 with insufficient_quota → billing, retryable=false.
        let openai_429 = r#"{"error":{"message":"You exceeded your current quota, please check your plan and billing details.","type":"insufficient_quota","code":"insufficient_quota"}}"#;
        let e = classify_error_response(429, openai_429, "OpenAI", "OpenAI");
        assert!(matches!(e, NarratorError::InsufficientCredit(_)));
        assert!(!is_rate_limit_error(&e), "billing must not be retried");
        assert!(e.to_string().to_lowercase().contains("billing"));

        // Anthropic: no-credit is a 400 invalid_request_error → billing.
        let anthropic_400 = r#"{"error":{"type":"invalid_request_error","message":"Your credit balance is too low to access the Anthropic API. Please go to Plans & Billing to upgrade or purchase credits."}}"#;
        assert!(matches!(
            classify_error_response(400, anthropic_400, "Claude", "Anthropic"),
            NarratorError::InsufficientCredit(_)
        ));

        // Anthropic 402 billing_error (empty/opaque body still classifies).
        assert!(matches!(
            classify_error_response(402, "", "Claude", "Anthropic"),
            NarratorError::InsufficientCredit(_)
        ));

        // A genuine per-minute rate limit stays retryable.
        let claude_429 = r#"{"error":{"type":"rate_limit_error","message":"Number of request tokens has exceeded your per-minute rate limit."}}"#;
        let e = classify_error_response(429, claude_429, "Claude", "Anthropic");
        assert!(matches!(e, NarratorError::RateLimited));
        assert!(is_rate_limit_error(&e));

        // 529 overloaded is transient/retryable.
        assert!(is_rate_limit_error(&classify_error_response(
            529,
            r#"{"error":{"type":"overloaded_error","message":"Overloaded"}}"#,
            "Claude",
            "Anthropic"
        )));

        // 401 → auth hint naming the right vendor, NOT retried, NOT billing.
        let e = classify_error_response(
            401,
            r#"{"error":{"message":"invalid x-api-key"}}"#,
            "Claude",
            "Anthropic",
        );
        assert!(matches!(e, NarratorError::ApiError(_)));
        assert!(e.to_string().contains("Anthropic API key"));
        assert!(!is_rate_limit_error(&e));

        // 400 model/param mismatch → generic ApiError, not billing.
        let e = classify_error_response(
            400,
            r#"{"error":{"message":"model: unknown model 'xyz'"}}"#,
            "OpenAI",
            "OpenAI",
        );
        assert!(matches!(e, NarratorError::ApiError(_)));
        assert!(e.to_string().contains("mismatch"));

        // 500 → generic server error, retryable via the 5xx text path.
        assert!(matches!(
            classify_error_response(500, "internal error", "Gemini", "Google"),
            NarratorError::ApiError(_)
        ));
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
            300.0,
            vec![],
            None,
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
            300.0,
            vec![],
            None,
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
            300.0,
            vec![],
            None,
            None,
            None,
        )
        .await;

        assert!(result.is_ok());
        let script = result.unwrap();
        assert_eq!(script.title, "Fenced");
    }

    // ── Contact-sheet tiling ───────────────────────────────────────────

    /// Write `n` tiny JPEGs and return frames pointing at them.
    fn tiled_test_frames(dir: &std::path::Path, n: usize) -> Vec<Frame> {
        (0..n)
            .map(|i| {
                let path = dir.join(format!("t{i}.jpg"));
                image::RgbImage::from_pixel(64, 36, image::Rgb([(i * 8) as u8, 0, 0]))
                    .save(&path)
                    .unwrap();
                Frame {
                    index: i,
                    timestamp_seconds: i as f64 * 2.0,
                    path,
                    width: 64,
                    height: 36,
                }
            })
            .collect()
    }

    fn count_images(parts: &[serde_json::Value]) -> usize {
        parts.iter().filter(|p| p["type"] == "image").count()
    }

    #[test]
    fn tiling_collapses_nine_frames_into_one_image_slot() {
        // This ratio is the entire point: image slots drive the chunk count.
        let dir = tempfile::tempdir().unwrap();
        let frames = tiled_test_frames(dir.path(), 9);

        let untiled = frame_content_parts(&frames, false).unwrap();
        assert_eq!(count_images(&untiled), 9);

        let tiled = frame_content_parts(&frames, true).unwrap();
        assert_eq!(count_images(&tiled), 1, "9 frames should be one sheet");
        assert_eq!(FRAMES_PER_SHEET, 9);
    }

    #[test]
    fn tiling_splits_across_sheets_when_frames_exceed_one_grid() {
        let dir = tempfile::tempdir().unwrap();
        let frames = tiled_test_frames(dir.path(), 20);
        let tiled = frame_content_parts(&frames, true).unwrap();
        // 20 frames at 9 per sheet → 3 sheets (9 + 9 + 2).
        assert_eq!(count_images(&tiled), 3);
    }

    #[test]
    fn tiling_puts_the_cell_mapping_before_its_image() {
        // The model must be told how to index the grid before it sees it.
        let dir = tempfile::tempdir().unwrap();
        let frames = tiled_test_frames(dir.path(), 4);
        let parts = frame_content_parts(&frames, true).unwrap();
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[1]["type"], "image");
        let text = parts[0]["text"].as_str().unwrap();
        assert!(text.contains("left to right"), "{text}");
        // Every frame must be addressable, or frame_refs break.
        for i in 0..4 {
            assert!(text.contains(&format!("frame {i} at")), "missing frame {i}");
        }
    }

    #[test]
    fn untiled_path_is_unchanged_by_the_tiling_feature() {
        // Regression guard: the default path must still emit one labelled image
        // per frame, in order.
        let dir = tempfile::tempdir().unwrap();
        let frames = tiled_test_frames(dir.path(), 3);
        let parts = frame_content_parts(&frames, false).unwrap();
        assert_eq!(parts.len(), 6, "text+image per frame");
        assert_eq!(parts[0]["text"], "[Frame 0 at 0.0s]");
        assert_eq!(parts[2]["text"], "[Frame 1 at 2.0s]");
        assert_eq!(parts[4]["text"], "[Frame 2 at 4.0s]");
    }

    #[test]
    fn both_paths_skip_a_frame_whose_file_vanished() {
        let dir = tempfile::tempdir().unwrap();
        let mut frames = tiled_test_frames(dir.path(), 2);
        frames.push(Frame {
            index: 99,
            timestamp_seconds: 50.0,
            path: dir.path().join("never-written.jpg"),
            width: 64,
            height: 36,
        });

        let untiled = frame_content_parts(&frames, false).unwrap();
        assert_eq!(count_images(&untiled), 2);

        let tiled = frame_content_parts(&frames, true).unwrap();
        assert_eq!(count_images(&tiled), 1);
        assert!(!tiled[0]["text"].as_str().unwrap().contains("frame 99"));
    }

    #[test]
    fn tiling_with_no_frames_emits_no_image_parts() {
        assert!(frame_content_parts(&[], true).unwrap().is_empty());
        assert!(frame_content_parts(&[], false).unwrap().is_empty());
    }

    // ── Silence-aware snapping ─────────────────────────────────────────

    fn span(start: f64, end: f64) -> SilenceSpan {
        SilenceSpan { start, end }
    }

    #[test]
    fn snap_moves_an_edge_into_a_nearby_clean_gap() {
        // Segment starts at 5.0, mid-speech; a 1s gap sits at 4.5-5.5.
        let spans = vec![span(4.5, 5.5)];
        let out = snap_to_silence(vec![seg(0, 5.0, 20.0, "hi")], &spans, 60.0);
        // 5.0 is already inside the gap, so nothing should move.
        assert_eq!(out[0].start_seconds, 5.0);

        // Now start just outside the gap.
        let out = snap_to_silence(vec![seg(0, 5.8, 20.0, "hi")], &spans, 60.0);
        // Snaps back into the gap, padded in from the 5.5 edge.
        assert!(
            out[0].start_seconds < 5.8 && out[0].start_seconds >= 5.0,
            "expected snap into 4.5-5.5, got {}",
            out[0].start_seconds
        );
    }

    #[test]
    fn snap_ignores_gaps_narrower_than_the_floor() {
        // A 100ms gap is mid-phrase — starting to speak there interrupts a word.
        let spans = vec![span(5.0, 5.1)];
        let out = snap_to_silence(vec![seg(0, 5.5, 20.0, "hi")], &spans, 60.0);
        assert_eq!(
            out[0].start_seconds, 5.5,
            "must not snap to a sub-150ms gap"
        );
    }

    #[test]
    fn snap_uses_a_usable_gap_when_no_clean_one_is_in_range() {
        // 250ms: inside the usable band (150-400ms) but not clean (>=400ms).
        let spans = vec![span(5.0, 5.25)];
        let out = snap_to_silence(vec![seg(0, 5.6, 20.0, "hi")], &spans, 60.0);
        assert!(
            out[0].start_seconds < 5.6,
            "a usable gap should still be used, got {}",
            out[0].start_seconds
        );
    }

    #[test]
    fn snap_prefers_a_clean_gap_over_a_closer_usable_one() {
        // A 200ms gap is nearer, but a 1s gap is also within the search window.
        // Cleanliness must win over proximity.
        let spans = vec![span(9.8, 10.0), span(10.4, 11.4)];
        let out = snap_to_silence(vec![seg(0, 10.2, 30.0, "hi")], &spans, 60.0);
        assert!(
            out[0].start_seconds > 10.2,
            "expected the clean 10.4-11.4 gap, got {}",
            out[0].start_seconds
        );
    }

    #[test]
    fn snap_refuses_to_move_an_edge_beyond_the_search_window() {
        // The only gap is 10s away — moving there would desync narration from
        // the visuals the model described.
        let spans = vec![span(30.0, 31.0)];
        let out = snap_to_silence(vec![seg(0, 5.0, 20.0, "hi")], &spans, 60.0);
        assert_eq!(out[0].start_seconds, 5.0);
        assert_eq!(out[0].end_seconds, 20.0);
    }

    #[test]
    fn snap_is_a_no_op_on_an_effectively_silent_source() {
        // A silent screencast reports one span covering the whole timeline.
        // Every edge would "snap" and nothing would improve — skip entirely.
        let spans = vec![span(0.0, 60.0)];
        assert!(is_effectively_silent(&spans, 60.0));
        let segs = vec![seg(0, 5.0, 20.0, "hi"), seg(1, 25.0, 40.0, "there")];
        let out = snap_to_silence(segs.clone(), &spans, 60.0);
        for (before, after) in segs.iter().zip(out.iter()) {
            assert_eq!(before.start_seconds, after.start_seconds);
            assert_eq!(before.end_seconds, after.end_seconds);
        }
    }

    #[test]
    fn snap_is_a_no_op_without_a_silence_map() {
        let segs = vec![seg(0, 5.0, 20.0, "hi")];
        let out = snap_to_silence(segs.clone(), &[], 60.0);
        assert_eq!(out[0].start_seconds, segs[0].start_seconds);
    }

    #[test]
    fn snap_never_inverts_or_collapses_a_segment() {
        // A gap sitting past the segment's own end must not drag the start
        // across it.
        let spans = vec![span(19.5, 20.5)];
        let out = snap_to_silence(vec![seg(0, 19.9, 20.1, "tiny")], &spans, 60.0);
        assert!(
            out[0].end_seconds > out[0].start_seconds,
            "segment inverted: {:?}..{:?}",
            out[0].start_seconds,
            out[0].end_seconds
        );
    }

    #[test]
    fn snap_keeps_edges_inside_the_video() {
        let spans = vec![span(58.0, 62.0)];
        let out = snap_to_silence(vec![seg(0, 50.0, 59.0, "end")], &spans, 60.0);
        assert!(
            out[0].end_seconds <= 60.0,
            "end ran past the video: {}",
            out[0].end_seconds
        );
    }

    #[test]
    fn effectively_silent_is_false_for_a_normally_speaking_source() {
        // ~13% silence over a minute: a real talking-head recording.
        let spans = vec![span(5.0, 6.0), span(20.0, 22.0), span(40.0, 45.0)];
        assert!(!is_effectively_silent(&spans, 60.0));
    }

    #[test]
    fn effectively_silent_handles_a_zero_length_video() {
        assert!(is_effectively_silent(&[], 0.0));
    }

    #[test]
    fn silence_prompt_block_lists_windows_and_caps_length() {
        let spans: Vec<SilenceSpan> = (0..100)
            .map(|i| span(i as f64 * 2.0, i as f64 * 2.0 + 0.5))
            .collect();
        let block = describe_silence_windows(&spans, 400.0);
        assert!(block.contains("EXISTING AUDIO"));
        // Capped so a chatty source can't flood the prompt.
        assert!(block.matches('-').count() <= 60, "window list not capped");
        assert!(block.contains("of 100 shown") || block.contains("of 100"));
    }

    #[test]
    fn silence_prompt_block_is_empty_when_it_would_mislead() {
        // Silent screencast: claiming "the source already has audio" is wrong.
        assert!(describe_silence_windows(&[span(0.0, 60.0)], 60.0).is_empty());
        assert!(describe_silence_windows(&[], 60.0).is_empty());
        // Only sub-floor gaps: nothing actionable to report.
        assert!(describe_silence_windows(&[span(5.0, 5.05)], 60.0).is_empty());
    }

    #[test]
    fn snapped_segments_survive_normalization() {
        // Snapping can push edges around; normalize_timeline is what restores
        // ordering and the minimum-length floor. Composed as the pipeline does.
        let spans = vec![span(9.6, 10.4), span(19.6, 20.4)];
        let segs = vec![seg(0, 10.0, 20.0, "one"), seg(1, 20.1, 30.0, "two")];
        let snapped = snap_to_silence(segs, &spans, 60.0);
        let out = normalize_timeline(snapped, 60.0);
        assert_eq!(out.len(), 2);
        assert!(out[0].end_seconds <= out[1].start_seconds + 1e-9);
        for s in &out {
            assert!(s.end_seconds > s.start_seconds);
        }
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

    // ── Duration authority (regression: v0.10.0 coverage collapse) ──────────
    //
    // Shipped bug: on the chunked path the merged script took
    // `total_duration_seconds` from the FIRST chunk's response. That chunk only
    // sees its own frames, so on a 220 s video it reported 53.2 s — and the final
    // `normalize_timeline` then deleted every segment starting past 54.2 s.
    // A 3:40 video shipped 5 segments covering 0:00–0:53.

    #[tokio::test]
    async fn a_short_first_chunk_duration_must_not_truncate_the_script() {
        // Four chunks over a 220 s video. Chunk 1 claims the video is only 53.2 s
        // long — exactly what the model did in production.
        let provider = ScriptedProvider::new(vec![
            &chunk_response_json("Demo", 53.2, &[(0.0, 8.5, "one"), (11.0, 19.0, "two")]),
            &chunk_response_json("Demo", 53.2, &[(60.0, 70.0, "three")]),
            &chunk_response_json("Demo", 53.2, &[(120.0, 130.0, "four")]),
            &chunk_response_json("Demo", 53.2, &[(190.0, 205.0, "five")]),
        ]);
        // 40 frames at 5.5 s → 4 chunks of 10, spanning 0–214 s.
        let msg = user_msg_with_frames(40, 5.5);

        let script = generate_narration(
            &provider,
            "sys",
            msg,
            "test",
            "en",
            220.3,
            Vec::new(),
            None,
            None,
            None,
        )
        .await
        .expect("generation succeeds");

        // The header must report the real video length, not the model's claim.
        assert!(
            (script.total_duration_seconds - 220.3).abs() < 0.01,
            "header says {:.1}s, expected the measured 220.3s",
            script.total_duration_seconds
        );

        // Segments from the later chunks must survive.
        let last_end = script.segments.last().map(|s| s.end_seconds).unwrap_or(0.0);
        assert!(
            last_end > 150.0,
            "narration stops at {last_end:.1}s of a 220s video — later chunks were dropped again"
        );
        assert!(
            script.segments.len() >= 4,
            "expected segments from all four chunks, got {}",
            script.segments.len()
        );
    }

    #[test]
    fn normalize_keeps_segments_when_bounded_by_the_real_duration() {
        // The mechanism itself: the bound is what decides whether later segments
        // live or die.
        let segs = vec![
            seg(0, 0.0, 8.0, "early"),
            seg(1, 60.0, 70.0, "middle"),
            seg(2, 190.0, 205.0, "late"),
        ];
        // Bounded by the model's short claim → the later two are deleted.
        assert_eq!(normalize_timeline(segs.clone(), 53.2).len(), 1);
        // Bounded by the real video length → all survive.
        assert_eq!(normalize_timeline(segs, 220.3).len(), 3);
    }

    #[tokio::test]
    async fn measured_duration_wins_on_the_single_call_path_too() {
        let provider = ScriptedProvider::new(vec![&chunk_response_json(
            "Demo",
            30.0,
            &[(0.0, 10.0, "one"), (100.0, 110.0, "two")],
        )]);
        let script = generate_narration(
            &provider,
            "sys",
            user_msg_with_frames(3, 5.0),
            "test",
            "en",
            220.3,
            Vec::new(),
            None,
            None,
            None,
        )
        .await
        .expect("generation succeeds");

        assert!((script.total_duration_seconds - 220.3).abs() < 0.01);
        assert_eq!(
            script.segments.len(),
            2,
            "the 100s segment must not be filtered by a 30s claim"
        );
    }

    #[tokio::test]
    async fn a_zero_measured_duration_falls_back_to_the_model_figure() {
        // CLI callers or a failed probe pass 0.0; behaviour must not regress to
        // deleting everything.
        let provider = ScriptedProvider::new(vec![&chunk_response_json(
            "Demo",
            60.0,
            &[(0.0, 10.0, "one"), (40.0, 50.0, "two")],
        )]);
        let script = generate_narration(
            &provider,
            "sys",
            user_msg_with_frames(3, 5.0),
            "test",
            "en",
            0.0,
            Vec::new(),
            None,
            None,
            None,
        )
        .await
        .expect("generation succeeds");
        assert_eq!(script.segments.len(), 2);
        assert!((script.total_duration_seconds - 60.0).abs() < 0.01);
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
        let result = generate_narration(
            &provider,
            "sys",
            msg,
            "test",
            "en",
            300.0,
            vec![],
            None,
            None,
            None,
        )
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

        let result = generate_narration(
            &provider,
            "sys",
            msg,
            "test",
            "en",
            300.0,
            vec![],
            None,
            None,
            None,
        )
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

        generate_narration(
            &provider,
            "sys",
            msg,
            "test",
            "en",
            300.0,
            vec![],
            None,
            None,
            None,
        )
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

        let result = generate_narration(
            &provider,
            "sys",
            msg,
            "test",
            "en",
            300.0,
            vec![],
            None,
            None,
            None,
        )
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

        let result = generate_narration(
            &provider,
            "sys",
            msg,
            "test",
            "en",
            300.0,
            vec![],
            None,
            None,
            None,
        )
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

        generate_narration(
            &provider,
            "sys",
            msg,
            "test",
            "en",
            300.0,
            vec![],
            None,
            None,
            None,
        )
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

    // ── contains_status_code: precision-matched HTTP status code detection ─
    // Regression: a naive `msg.contains("429")` matched things like
    // "request 429 of 1000" or "5000 characters", which could spuriously
    // trigger retry or rewrite an error message. The mirror frontend bug
    // surfaced as "API server is temporarily unavailable" on a description
    // length validation. These tests pin the boundary behavior.

    #[test]
    fn test_contains_status_code_isolated_match() {
        assert!(contains_status_code("HTTP 429 too many requests", 429));
        assert!(contains_status_code("status: 429", 429));
        assert!(contains_status_code("429", 429));
        assert!(contains_status_code("(429)", 429));
    }

    #[test]
    fn test_contains_status_code_rejects_digit_neighbors() {
        // The whole point of the helper: don't match digit substrings.
        assert!(!contains_status_code("processed 4290 frames", 429));
        assert!(!contains_status_code("request 1429 failed", 429));
        assert!(!contains_status_code("1429", 429));
        assert!(!contains_status_code("4290", 429));
    }

    #[test]
    fn test_contains_status_code_500_does_not_match_5000() {
        // The original frontend bug was that "Description must be 5000 chars"
        // matched `lower.includes("500")`. The boundary check rejects this.
        assert!(!contains_status_code("description must be 5000 chars", 500));
        // "500 chars" still matches because 500 IS an isolated number — the
        // helper is intentionally pure. The frontend layer handles the
        // validation-message case by pass-through-matching "characters or
        // fewer" before any digit-based check.
        assert!(contains_status_code("title must be 500 characters", 500));
    }

    #[test]
    fn test_contains_status_code_short_input() {
        assert!(!contains_status_code("", 429));
        assert!(!contains_status_code("42", 429));
        assert!(!contains_status_code("hi", 500));
    }

    #[test]
    fn test_contains_status_code_at_string_boundaries() {
        // Match at start and end of string (no neighbor to fail the check).
        assert!(contains_status_code("429 ", 429));
        assert!(contains_status_code(" 429", 429));
        assert!(contains_status_code("429", 429));
    }

    // ── is_rate_limit_error ────────────────────────────────────────────────

    #[test]
    fn test_is_rate_limit_error_explicit_variant() {
        // The cleanest signal: every provider now returns this on 429/529.
        assert!(is_rate_limit_error(&NarratorError::RateLimited));
    }

    #[test]
    fn test_is_rate_limit_error_status_codes() {
        let err = NarratorError::ApiError("Claude API error (HTTP 429). ...".into());
        assert!(is_rate_limit_error(&err));
        let err = NarratorError::ApiError("Claude API error (HTTP 529). ...".into());
        assert!(is_rate_limit_error(&err));
    }

    #[test]
    fn test_is_rate_limit_error_text_variants() {
        assert!(is_rate_limit_error(&NarratorError::ApiError(
            "rate limit exceeded".into()
        )));
        assert!(is_rate_limit_error(&NarratorError::ApiError(
            "TOO MANY REQUESTS".into()
        )));
        assert!(is_rate_limit_error(&NarratorError::ApiError(
            "model is overloaded".into()
        )));
    }

    #[test]
    fn test_is_rate_limit_error_does_not_match_unrelated_digits() {
        // Regression: "processed 4290 frames" no longer matches.
        assert!(!is_rate_limit_error(&NarratorError::ApiError(
            "processed 4290 frames".into()
        )));
        assert!(!is_rate_limit_error(&NarratorError::ApiError(
            "video has 5290 frames".into()
        )));
        assert!(!is_rate_limit_error(&NarratorError::ApiError(
            "ffmpeg returned exit code 0".into()
        )));
    }

    #[test]
    fn test_is_rate_limit_error_does_not_match_other_status_codes() {
        let err = NarratorError::ApiError("HTTP 500 internal server error".into());
        assert!(!is_rate_limit_error(&err));
        let err = NarratorError::ApiError("HTTP 401 unauthorized".into());
        assert!(!is_rate_limit_error(&err));
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

        let result = generate_narration(
            &provider,
            "sys",
            msg,
            "test",
            "en",
            300.0,
            vec![],
            None,
            None,
            None,
        )
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

        let result = generate_narration(
            &provider,
            "sys",
            msg,
            "test",
            "en",
            300.0,
            vec![],
            None,
            None,
            None,
        )
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

        let result = generate_narration(
            &provider,
            "sys",
            msg,
            "test",
            "ja",
            300.0,
            vec![],
            None,
            None,
            None,
        )
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

        generate_narration(
            &provider,
            "sys",
            msg,
            "test",
            "en",
            300.0,
            vec![],
            None,
            None,
            None,
        )
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

        generate_narration(
            &provider,
            "sys",
            msg,
            "test",
            "en",
            300.0,
            vec![],
            None,
            None,
            None,
        )
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

        let result = generate_narration(
            &provider,
            "sys",
            msg,
            "test",
            "en",
            300.0,
            vec![],
            None,
            None,
            None,
        )
        .await;
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

        let err = generate_narration(
            &provider,
            "sys",
            msg,
            "test",
            "en",
            300.0,
            vec![],
            None,
            None,
            None,
        )
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

        generate_narration(
            &provider,
            "sys",
            msg,
            "test",
            "en",
            300.0,
            vec![],
            None,
            None,
            None,
        )
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

        let result = generate_narration(
            &provider,
            "sys",
            msg,
            "test",
            "en",
            300.0,
            vec![],
            None,
            None,
            None,
        )
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

        generate_narration(
            &provider,
            "sys",
            msg,
            "test",
            "en",
            300.0,
            vec![],
            None,
            Some(cb),
            None,
        )
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

        // Chunks must stop short of the top of the band. The remainder is
        // reserved for polish and the final passes — when chunks owned the whole
        // band the bar hit its ceiling and then froze for up to 90s, because the
        // frontend clamps monotonic-forward and every later report was lower.
        // All but the final (finalize) tick come from the chunk loop.
        let max_chunk_tick = ticks[..ticks.len() - 1]
            .iter()
            .map(|(f, _)| *f)
            .fold(0.0_f64, f64::max);
        assert!(
            max_chunk_tick <= CHUNK_SPAN + 1e-6,
            "chunk ticks reached {max_chunk_tick}, must stay within CHUNK_SPAN {CHUNK_SPAN}"
        );
        // And the final tick must be the reserved finalize step, above the chunk
        // ceiling so the bar keeps advancing.
        let (last_frac, _) = ticks.last().unwrap();
        assert!(
            (*last_frac - FINALIZE_FRACTION).abs() < 1e-6,
            "last tick should be FINALIZE_FRACTION {FINALIZE_FRACTION}, got {last_frac}"
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

        generate_narration(
            &provider,
            "sys",
            msg,
            "test",
            "en",
            300.0,
            resume,
            None,
            Some(cb),
            None,
        )
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
            chapters: None,
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
            chapters: None,
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
            chapters: None,
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
            chapters: None,
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
            Some("Always use formal product voice"),
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

    // ── Structured output: wire shapes ──────────────────────────────────────
    //
    // Each provider spells schema enforcement differently and gets it wrong as
    // a 400, not a soft degrade. These assert the exact request shape without
    // touching the network.

    #[test]
    fn claude_schema_forces_a_single_tool_call() {
        let schema = response_schema::narration_script();
        let mut body = build_claude_body(
            "claude-sonnet-5",
            0.7,
            ReasoningEffort::Balanced,
            "sys",
            json!([{"type": "text", "text": "hi"}]),
        );
        apply_claude_schema(&mut body, &schema);

        let tools = body["tools"].as_array().expect("tools array");
        assert_eq!(tools.len(), 1, "exactly one tool, or the model may choose");
        assert_eq!(tools[0]["name"], schema.name);
        assert!(tools[0]["description"]
            .as_str()
            .is_some_and(|d| !d.is_empty()));
        // The schema must land under `input_schema`, not `schema`.
        assert_eq!(tools[0]["input_schema"]["type"], "object");
        assert!(tools[0]["input_schema"]["properties"]["segments"].is_object());

        // Forced, not merely offered — `auto` would let the model reply in prose.
        assert_eq!(body["tool_choice"]["type"], "tool");
        assert_eq!(body["tool_choice"]["name"], schema.name);

        // Enforcement must not disturb the capability-matrix decisions.
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert!(
            body.get("temperature").is_none(),
            "sonnet-5 rejects sampling params"
        );
    }

    #[test]
    fn openai_schema_uses_strict_json_schema() {
        let schema = response_schema::narration_script();
        let mut body = build_openai_body(
            "gpt-5",
            0.7,
            ReasoningEffort::Balanced,
            "sys",
            json!([{"type": "text", "text": "hi"}]),
        );
        apply_openai_schema(&mut body, &schema);

        assert_eq!(body["response_format"]["type"], "json_schema");
        let js = &body["response_format"]["json_schema"];
        assert_eq!(js["name"], schema.name);
        // Without `strict` the schema is a hint, which defeats the purpose.
        assert_eq!(js["strict"], true);
        assert_eq!(js["schema"]["type"], "object");
        assert_eq!(js["schema"]["additionalProperties"], false);
    }

    #[test]
    fn gemini_schema_sets_response_schema_in_its_own_dialect() {
        let schema = response_schema::narration_script();
        let mut body = build_gemini_body(
            "gemini-3-pro",
            0.7,
            ReasoningEffort::Balanced,
            "sys",
            vec![json!({"text": "hi"})],
        );
        apply_gemini_schema(&mut body, &schema);

        let cfg = &body["generationConfig"];
        // Both are needed: mime type says "JSON", schema says "this JSON".
        assert_eq!(cfg["responseMimeType"], "application/json");
        assert_eq!(cfg["responseSchema"]["type"], "object");
        assert!(cfg["responseSchema"]["properties"]["segments"].is_object());
        // `additionalProperties` is a hard 400 on Gemini.
        assert!(
            !cfg["responseSchema"]
                .to_string()
                .contains("additionalProperties"),
            "unsupported keyword reached the Gemini request"
        );
        // Pre-existing thinking config must survive.
        assert_eq!(cfg["thinkingLevel"], "medium");
    }

    #[test]
    fn gemini_schema_does_not_clobber_sibling_generation_config() {
        let mut body = build_gemini_body(
            "gemini-2.5-pro",
            0.4,
            ReasoningEffort::Fast,
            "sys",
            vec![json!({"text": "hi"})],
        );
        apply_gemini_schema(&mut body, &response_schema::critique());
        // Compared with tolerance: the f32 temperature widens to f64 in JSON.
        let temp = body["generationConfig"]["temperature"]
            .as_f64()
            .expect("temperature");
        assert!((temp - 0.4).abs() < 1e-6, "temperature was {temp}");
        assert!(body["generationConfig"]["maxOutputTokens"].is_number());
        assert!(body["generationConfig"]["responseSchema"].is_object());
    }

    // ── Structured output: response extraction ──────────────────────────────

    #[test]
    fn claude_payload_comes_from_the_tool_use_block() {
        let response = json!({
            "stop_reason": "tool_use",
            "content": [
                {"type": "text", "text": "let me think about this"},
                {"type": "tool_use", "name": "narration_script",
                 "input": {"title": "From tool", "segments": []}}
            ]
        });
        let payload = extract_claude_payload(&response, Some("narration_script"));
        let parsed: serde_json::Value = serde_json::from_str(&payload).expect("valid JSON");
        // The tool input wins over the chatty text block that precedes it.
        assert_eq!(parsed["title"], "From tool");
    }

    #[test]
    fn claude_payload_ignores_a_tool_use_block_with_another_name() {
        let response = json!({
            "stop_reason": "tool_use",
            "content": [
                {"type": "tool_use", "name": "some_other_tool", "input": {"title": "wrong"}},
                {"type": "text", "text": "{\"title\":\"from text\"}"}
            ]
        });
        let payload = extract_claude_payload(&response, Some("narration_script"));
        let parsed: serde_json::Value = serde_json::from_str(&payload).expect("valid JSON");
        assert_eq!(parsed["title"], "from text");
    }

    #[test]
    fn claude_payload_falls_back_to_text_when_the_tool_call_is_missing() {
        // A response truncated by max_tokens can arrive with no tool_use block.
        // Falling back to text gives the caller a real parse error instead of an
        // empty string that reads as a successful empty script.
        let response = json!({
            "stop_reason": "max_tokens",
            "content": [{"type": "text", "text": "{\"title\":\"partial\"}"}]
        });
        let payload = extract_claude_payload(&response, Some("narration_script"));
        assert_eq!(payload, "{\"title\":\"partial\"}");
    }

    #[test]
    fn claude_payload_reads_text_when_no_schema_was_requested() {
        let response = json!({
            "content": [{"type": "text", "text": "plain prose reply"}]
        });
        assert_eq!(extract_claude_payload(&response, None), "plain prose reply");
    }

    #[test]
    fn claude_payload_is_empty_when_there_is_no_usable_block() {
        let response = json!({ "content": [] });
        assert_eq!(extract_claude_payload(&response, None), "");
        assert_eq!(
            extract_claude_payload(&json!({}), Some("narration_script")),
            ""
        );
    }

    // ── Content conversion ──────────────────────────────────────────────────

    #[test]
    fn openai_conversion_maps_images_to_data_urls() {
        let claude_content = json!([
            {"type": "text", "text": "frame 0"},
            {"type": "image", "source": {"media_type": "image/jpeg", "data": "QUJD"}}
        ]);
        let converted = claude_content_to_openai(&claude_content);
        let parts = converted.as_array().expect("array");
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[0]["text"], "frame 0");
        assert_eq!(parts[1]["type"], "image_url");
        assert_eq!(parts[1]["image_url"]["url"], "data:image/jpeg;base64,QUJD");
    }

    #[test]
    fn gemini_conversion_maps_images_to_inline_data() {
        let claude_content = json!([
            {"type": "text", "text": "frame 0"},
            {"type": "image", "source": {"media_type": "image/jpeg", "data": "QUJD"}}
        ]);
        let parts = claude_content_to_gemini(&claude_content);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["text"], "frame 0");
        assert_eq!(parts[1]["inlineData"]["data"], "QUJD");
        assert_eq!(parts[1]["inlineData"]["mimeType"], "image/jpeg");
    }

    #[test]
    fn conversions_accept_a_bare_string_message() {
        // The polish / translate / refine paths pass a JSON string, not an array.
        let msg = json!("just text");
        let openai = claude_content_to_openai(&msg);
        assert_eq!(openai[0]["text"], "just text");
        let gemini = claude_content_to_gemini(&msg);
        assert_eq!(gemini[0]["text"], "just text");
    }

    // ── Schema threading ────────────────────────────────────────────────────

    /// Records whether the schema-aware entry point was used, and which schema.
    struct SchemaSpy {
        response: String,
        saw_schema: std::sync::Mutex<Option<String>>,
        plain_calls: std::sync::atomic::AtomicUsize,
    }

    impl SchemaSpy {
        fn new(response: &str) -> Self {
            Self {
                response: response.to_string(),
                saw_schema: std::sync::Mutex::new(None),
                plain_calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl AiProvider for SchemaSpy {
        async fn generate(
            &self,
            _system_prompt: &str,
            _user_message: serde_json::Value,
        ) -> Result<String, NarratorError> {
            self.plain_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.response.clone())
        }
        async fn generate_with_schema(
            &self,
            _system_prompt: &str,
            _user_message: serde_json::Value,
            schema: &ResponseSchema,
        ) -> Result<String, NarratorError> {
            *self.saw_schema.lock().unwrap() = Some(schema.name.to_string());
            Ok(self.response.clone())
        }
        fn name(&self) -> &str {
            "spy"
        }
        fn model(&self) -> &str {
            "spy-v1"
        }
    }

    #[tokio::test]
    async fn narration_generation_requests_the_script_schema() {
        let spy = SchemaSpy::new(&chunk_response_json(
            "Spy",
            10.0,
            &[(0.0, 5.0, "one"), (5.0, 10.0, "two")],
        ));
        generate_narration(
            &spy,
            "sys",
            json!([{"type": "text", "text": "ctx"}]),
            "technical",
            "en",
            10.0,
            Vec::new(),
            None,
            None,
            None,
        )
        .await
        .expect("generation succeeds");

        assert_eq!(
            spy.saw_schema.lock().unwrap().as_deref(),
            Some("narration_script")
        );
        assert_eq!(
            spy.plain_calls.load(Ordering::SeqCst),
            0,
            "narration must not fall back to the unstructured entry point"
        );
    }

    #[tokio::test]
    async fn refine_segment_stays_unstructured() {
        // This path returns a bare sentence; forcing an object schema would
        // break it.
        let spy = SchemaSpy::new("A tighter sentence.");
        let out = refine_segment(&spy, "old text", "make it shorter", "")
            .await
            .expect("refine succeeds");
        assert_eq!(out, "A tighter sentence.");
        assert!(spy.saw_schema.lock().unwrap().is_none());
        assert_eq!(spy.plain_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_provider_without_schema_support_still_works() {
        // The default trait method delegates to `generate`, so a provider (or
        // test double) that never overrides it keeps functioning.
        let provider = ScriptedProvider::new(vec![&chunk_response_json(
            "Fallback",
            10.0,
            &[(0.0, 10.0, "only")],
        )]);
        let script = generate_narration(
            &provider,
            "sys",
            json!([{"type": "text", "text": "ctx"}]),
            "technical",
            "en",
            10.0,
            Vec::new(),
            None,
            None,
            None,
        )
        .await
        .expect("unstructured provider still generates");
        assert_eq!(script.title, "Fallback");
    }
}
