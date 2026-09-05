//! Response schemas for provider-enforced structured output.
//!
//! Every AI call in this app expects JSON back. Asking for it in prose
//! ("Return ONLY the JSON, no markdown code fences") works most of the time,
//! and the tolerant parser in `ai_client` mops up fences and stray prose — but
//! a malformed response still costs a full re-generation, and on a chunked
//! 300-frame job that is one wasted call out of thirty.
//!
//! All three providers can enforce a schema at the API level instead:
//!   - Anthropic: a single tool with `input_schema`, forced via `tool_choice`.
//!   - OpenAI: `response_format: {type: "json_schema", strict: true}`.
//!   - Gemini: `generationConfig.responseSchema`.
//!
//! The canonical schemas here are written to satisfy the *strictest* consumer
//! (OpenAI strict mode), because that dialect is a subset of what the other two
//! accept:
//!   - every declared property appears in `required`
//!   - every object sets `additionalProperties: false`
//!   - no `$ref`, no `oneOf`, no open-ended maps
//!
//! Gemini's `responseSchema` is an OpenAPI 3.0 subset that *rejects*
//! `additionalProperties`, so it gets the canonical form run through
//! [`to_gemini_dialect`] rather than a hand-maintained second copy — one source
//! of truth, mechanically adapted.
//!
//! Fields the model must not invent are deliberately absent from these
//! schemas: `voice_override` and `speech_rate_report` are app-owned state
//! populated after generation, and every omitted field has a serde default on
//! the Rust side, so absence deserializes cleanly.

use serde_json::{json, Value};

/// A named schema for one response shape, in canonical (OpenAI-strict) form.
#[derive(Debug, Clone)]
pub struct ResponseSchema {
    /// Schema identifier. Doubles as the Anthropic tool name, so it must match
    /// `^[a-zA-Z0-9_-]{1,64}$`.
    pub name: &'static str,
    /// Sent to Anthropic as the tool description. The model reads this, so it
    /// should describe the *intent*, not restate the field list.
    pub description: &'static str,
    /// The JSON Schema itself.
    pub schema: Value,
}

/// Schema for a full [`crate::models::NarrationScript`].
///
/// Shared by five call paths that all deserialize into `NarrationScript`:
/// first-pass generation, per-chunk generation, the polish pass, translation,
/// and whole-script refinement.
pub fn narration_script() -> ResponseSchema {
    ResponseSchema {
        name: "narration_script",
        description: "Emit the timed narration script for the video.",
        schema: json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["title", "total_duration_seconds", "segments", "metadata"],
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Short title for the video."
                },
                "total_duration_seconds": {
                    "type": "number",
                    "description": "Total video duration in seconds."
                },
                "segments": {
                    "type": "array",
                    "description": "Narration segments in ascending time order.",
                    "items": segment_schema()
                },
                "metadata": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["style", "language", "provider", "model", "generated_at"],
                    "properties": {
                        "style": {"type": "string"},
                        "language": {"type": "string"},
                        "provider": {"type": "string"},
                        "model": {"type": "string"},
                        "generated_at": {
                            "type": "string",
                            "description": "ISO 8601 timestamp."
                        }
                    }
                }
            }
        }),
    }
}

/// One narration segment. Mirrors [`crate::models::Segment`] minus the
/// app-owned `voice_override`.
fn segment_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": [
            "index", "start_seconds", "end_seconds", "text",
            "visual_description", "emphasis", "pace", "pause_after_ms", "frame_refs"
        ],
        "properties": {
            "index": {
                "type": "integer",
                "minimum": 0,
                "description": "Zero-based position in the segment list."
            },
            "start_seconds": {
                "type": "number",
                "minimum": 0,
                "description": "Segment start, in seconds from video start."
            },
            "end_seconds": {
                "type": "number",
                "minimum": 0,
                "description": "Segment end, in seconds. Must exceed start_seconds."
            },
            "text": {
                "type": "string",
                "description": "Plain speakable narration. No markup, tags, or \
                                directives such as [pause] — this string is sent \
                                verbatim to a text-to-speech engine."
            },
            "visual_description": {
                "type": "string",
                "description": "What is on screen during this segment."
            },
            "emphasis": {
                "type": "array",
                "description": "Words or short phrases from `text` to stress.",
                "items": {"type": "string"}
            },
            "pace": {
                "type": "string",
                "enum": ["slow", "medium", "fast"],
                "description": "Delivery speed for this segment."
            },
            "pause_after_ms": {
                "type": "integer",
                "minimum": 0,
                "description": "Silence to insert after this segment, in milliseconds."
            },
            "frame_refs": {
                "type": "array",
                "description": "Indices of the frames this segment describes.",
                "items": {"type": "integer", "minimum": 0}
            }
        }
    })
}

/// Schema for the self-critique pass response.
///
/// The critique pass re-shows frames for specific segments and asks which ones
/// describe something not actually on screen.
pub fn critique() -> ResponseSchema {
    ResponseSchema {
        name: "critique_result",
        description: "Report narration segments that do not match their frames.",
        schema: json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["mismatches"],
            "properties": {
                "mismatches": {
                    "type": "array",
                    "description": "One entry per mismatched segment. Empty when \
                                    every segment matches its frames.",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["segment_index", "reason", "suggestion"],
                        "properties": {
                            "segment_index": {
                                "type": "integer",
                                "minimum": 0,
                                "description": "Index of the offending segment."
                            },
                            "reason": {
                                "type": "string",
                                "description": "What the narration got wrong."
                            },
                            "suggestion": {
                                "type": "string",
                                "description": "Concrete rewrite guidance."
                            }
                        }
                    }
                }
            }
        }),
    }
}

/// Schema for the auto-chapter pass.
///
/// The model returns only a starting *segment index* per chapter, never a
/// timestamp: it already has the segment list, and letting it invent seconds
/// invites values that drift off the real segment boundaries. The caller looks
/// the timestamp up from the segment it names.
pub fn chapters() -> ResponseSchema {
    ResponseSchema {
        name: "chapter_list",
        description: "Group consecutive narration segments into named chapters.",
        schema: json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["chapters"],
            "properties": {
                "chapters": {
                    "type": "array",
                    "description": "Chapters in timeline order. Empty when the \
                                    video is too short or too uniform to divide.",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["title", "start_segment"],
                        "properties": {
                            "title": {
                                "type": "string",
                                "description": "Short descriptive label, 2-6 words, \
                                                no numbering and no trailing period."
                            },
                            "start_segment": {
                                "type": "integer",
                                "minimum": 0,
                                "description": "Index of the first narration segment \
                                                belonging to this chapter."
                            }
                        }
                    }
                }
            }
        }),
    }
}

/// Schema for the frame-selection survey pass.
///
/// The survey call picks *timestamps*, nothing else — asking for narration in
/// the same call produces a worse version of both.
pub fn frame_selection() -> ResponseSchema {
    ResponseSchema {
        name: "frame_selection",
        description: "Choose the video timestamps that deserve a closer look.",
        schema: json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["moments"],
            "properties": {
                "moments": {
                    "type": "array",
                    "description": "Selected moments, in ascending time order.",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["timestamp", "reason"],
                        "properties": {
                            "timestamp": {
                                "type": "number",
                                "minimum": 0,
                                "description": "Seconds from the start of the video."
                            },
                            "reason": {
                                "type": "string",
                                "description": "Short, concrete reason this moment matters."
                            }
                        }
                    }
                }
            }
        }),
    }
}

/// Rewrite a canonical schema into Gemini's `responseSchema` dialect.
///
/// Gemini accepts an OpenAPI 3.0 subset, which differs from JSON Schema in
/// ways that are 400s rather than warnings:
///   - `additionalProperties` is not a recognised key.
///   - Nullability is `nullable: true`, not a `["string", "null"]` type union.
///   - Validation keywords it does not implement (`minimum`, `$schema`) are
///     dropped rather than risked.
///
/// Applied mechanically so the canonical schema above stays the single source
/// of truth.
pub fn to_gemini_dialect(schema: &Value) -> Value {
    match schema {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, value) in map {
                match key.as_str() {
                    // Not part of the OpenAPI subset Gemini implements.
                    "additionalProperties" | "$schema" | "strict" | "minimum" | "maximum" => {}
                    // A `["string", "null"]` union becomes `nullable: true`.
                    "type" => match value {
                        Value::Array(variants) => {
                            let nullable = variants.iter().any(|v| v == "null");
                            let concrete = variants
                                .iter()
                                .find(|v| *v != "null")
                                .cloned()
                                .unwrap_or(Value::String("string".into()));
                            out.insert("type".into(), concrete);
                            if nullable {
                                out.insert("nullable".into(), Value::Bool(true));
                            }
                        }
                        other => {
                            out.insert("type".into(), other.clone());
                        }
                    },
                    _ => {
                        out.insert(key.clone(), to_gemini_dialect(value));
                    }
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(to_gemini_dialect).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::NarrationScript;

    /// Walk every object in a schema tree and assert a property holds.
    fn for_each_object(schema: &Value, f: &mut impl FnMut(&serde_json::Map<String, Value>)) {
        match schema {
            Value::Object(map) => {
                if map.get("type").and_then(|t| t.as_str()) == Some("object") {
                    f(map);
                }
                for value in map.values() {
                    for_each_object(value, f);
                }
            }
            Value::Array(items) => {
                for item in items {
                    for_each_object(item, f);
                }
            }
            _ => {}
        }
    }

    /// OpenAI strict mode requires that every declared property is also listed
    /// in `required`. A drifted schema is a 400 at generation time, so this is
    /// checked structurally rather than trusted by eye.
    #[test]
    fn canonical_schemas_satisfy_openai_strict_mode() {
        for named in [narration_script(), critique(), frame_selection()] {
            let mut checked = 0;
            for_each_object(&named.schema, &mut |obj| {
                checked += 1;
                assert_eq!(
                    obj.get("additionalProperties"),
                    Some(&Value::Bool(false)),
                    "{}: every object needs additionalProperties:false",
                    named.name
                );
                let props: Vec<&String> = obj
                    .get("properties")
                    .and_then(|p| p.as_object())
                    .map(|p| p.keys().collect())
                    .unwrap_or_default();
                let required: Vec<&str> = obj
                    .get("required")
                    .and_then(|r| r.as_array())
                    .map(|r| r.iter().filter_map(|v| v.as_str()).collect())
                    .unwrap_or_default();
                for prop in props {
                    assert!(
                        required.contains(&prop.as_str()),
                        "{}: property `{prop}` is declared but not required",
                        named.name
                    );
                }
            });
            assert!(checked >= 2, "{}: expected nested objects", named.name);
        }
    }

    /// Anthropic tool names are constrained to `^[a-zA-Z0-9_-]{1,64}$`.
    #[test]
    fn schema_names_are_valid_anthropic_tool_names() {
        for named in [narration_script(), critique(), frame_selection()] {
            assert!(!named.name.is_empty() && named.name.len() <= 64);
            assert!(
                named
                    .name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
                "invalid tool name: {}",
                named.name
            );
            assert!(!named.description.is_empty());
        }
    }

    /// The whole point of the schema is that a conforming response
    /// deserializes into the real type. Build a minimal instance by hand and
    /// prove `NarrationScript` accepts it.
    #[test]
    fn schema_conforming_payload_deserializes_into_narration_script() {
        let payload = json!({
            "title": "Demo",
            "total_duration_seconds": 12.0,
            "segments": [{
                "index": 0,
                "start_seconds": 0.0,
                "end_seconds": 5.0,
                "text": "Opening line.",
                "visual_description": "Terminal window.",
                "emphasis": ["Opening"],
                "pace": "medium",
                "pause_after_ms": 300,
                "frame_refs": [0, 1]
            }],
            "metadata": {
                "style": "technical",
                "language": "en",
                "provider": "claude",
                "model": "claude-sonnet-5",
                "generated_at": "2026-01-01T00:00:00Z"
            }
        });

        let script: NarrationScript =
            serde_json::from_value(payload).expect("schema-shaped payload must deserialize");
        assert_eq!(script.segments.len(), 1);
        assert_eq!(script.segments[0].text, "Opening line.");
        // Omitted from the schema on purpose — app-owned, defaulted by serde.
        assert!(script.segments[0].voice_override.is_none());
        assert!(script.speech_rate_report.is_none());
    }

    /// The `pace` enum in the schema must match the `Pace` serde
    /// representation, or the model emits a value serde rejects.
    #[test]
    fn pace_enum_matches_serde_representation() {
        let schema = narration_script().schema;
        let pace = &schema["properties"]["segments"]["items"]["properties"]["pace"]["enum"];
        let variants: Vec<&str> = pace
            .as_array()
            .expect("pace enum")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(variants, vec!["slow", "medium", "fast"]);
        for variant in variants {
            let quoted = format!("\"{variant}\"");
            serde_json::from_str::<crate::models::Pace>(&quoted)
                .unwrap_or_else(|e| panic!("Pace rejects schema variant {variant}: {e}"));
        }
    }

    #[test]
    fn gemini_dialect_drops_unsupported_keywords() {
        let converted = to_gemini_dialect(&narration_script().schema);
        let mut objects = 0;
        for_each_object(&converted, &mut |obj| {
            objects += 1;
            assert!(
                !obj.contains_key("additionalProperties"),
                "additionalProperties survived conversion"
            );
        });
        assert!(objects >= 2);

        let as_text = converted.to_string();
        assert!(!as_text.contains("additionalProperties"));
        assert!(!as_text.contains("minimum"));
        // Structure the model needs must survive.
        assert!(as_text.contains("segments"));
        assert!(as_text.contains("start_seconds"));
        assert_eq!(converted["type"], "object");
        assert_eq!(converted["properties"]["segments"]["type"], "array");
    }

    #[test]
    fn gemini_dialect_converts_nullable_type_unions() {
        let input = json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["maybe"],
            "properties": {
                "maybe": {"type": ["string", "null"]}
            }
        });
        let out = to_gemini_dialect(&input);
        assert_eq!(out["properties"]["maybe"]["type"], "string");
        assert_eq!(out["properties"]["maybe"]["nullable"], true);
    }

    #[test]
    fn gemini_dialect_preserves_required_and_enums() {
        let converted = to_gemini_dialect(&narration_script().schema);
        let required: Vec<&str> = converted["required"]
            .as_array()
            .expect("required")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(required.contains(&"segments"));
        assert!(required.contains(&"metadata"));
        let pace = &converted["properties"]["segments"]["items"]["properties"]["pace"]["enum"];
        assert!(pace.is_array(), "enum must survive dialect conversion");
    }

    #[test]
    fn critique_schema_matches_the_parser_contract() {
        let schema = critique().schema;
        // `parse_critique_response` reads `mismatches[].segment_index` and
        // `.suggestion`; if the schema stops requiring them the parser silently
        // skips every entry.
        let item = &schema["properties"]["mismatches"]["items"];
        let required: Vec<&str> = item["required"]
            .as_array()
            .expect("required")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(required.contains(&"segment_index"));
        assert!(required.contains(&"suggestion"));
        assert_eq!(item["properties"]["segment_index"]["type"], "integer");
    }
}
