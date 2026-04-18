//! Export formatters for narration scripts in JSON, SRT, VTT, TXT, MD, and SSML.

use crate::models::{NarrationScript, Pace};

pub fn export_json(script: &NarrationScript) -> String {
    serde_json::to_string_pretty(script).unwrap_or_default()
}

pub fn export_srt(script: &NarrationScript) -> String {
    let mut output = String::new();

    for (i, segment) in script.segments.iter().enumerate() {
        output.push_str(&format!("{}\n", i + 1));
        output.push_str(&format!(
            "{} --> {}\n",
            format_srt_time(segment.start_seconds),
            format_srt_time(segment.end_seconds)
        ));
        output.push_str(&segment.text);
        output.push_str("\n\n");
    }

    output
}

pub fn export_vtt(script: &NarrationScript) -> String {
    let mut output = String::from("WEBVTT\n\n");

    for segment in &script.segments {
        output.push_str(&format!(
            "{} --> {}\n",
            format_vtt_time(segment.start_seconds),
            format_vtt_time(segment.end_seconds)
        ));
        output.push_str(&segment.text);
        output.push_str("\n\n");
    }

    output
}

pub fn export_txt(script: &NarrationScript) -> String {
    let mut output = String::new();

    for segment in &script.segments {
        output.push_str(&format!(
            "[{} - {}]\n",
            format_human_time(segment.start_seconds),
            format_human_time(segment.end_seconds)
        ));
        output.push_str(&segment.text);
        output.push_str("\n\n");
    }

    output
}

pub fn export_markdown(script: &NarrationScript) -> String {
    let mut output = String::new();

    output.push_str(&format!("# {}\n\n", script.title));
    output.push_str(&format!(
        "**Duration:** {:.0}s | **Style:** {} | **Language:** {}\n\n",
        script.total_duration_seconds, script.metadata.style, script.metadata.language
    ));
    output.push_str("---\n\n");
    output.push_str("| # | Time | Text | Pace |\n");
    output.push_str("|---|------|------|------|\n");

    for segment in &script.segments {
        let time = format!(
            "{} - {}",
            format_human_time(segment.start_seconds),
            format_human_time(segment.end_seconds)
        );
        // Escape pipes in text for markdown table
        let text = segment.text.replace('|', "\\|").replace('\n', " ");
        output.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            segment.index, time, text, segment.pace
        ));
    }

    output.push_str("\n---\n\n## Full Script\n\n");

    for segment in &script.segments {
        output.push_str(&format!(
            "**[{} - {}]** {}\n\n",
            format_human_time(segment.start_seconds),
            format_human_time(segment.end_seconds),
            segment.text
        ));
    }

    output
}

pub fn export_ssml(script: &NarrationScript) -> String {
    let lang = match script.metadata.language.as_str() {
        "en" => "en-US",
        "ja" => "ja-JP",
        "de" => "de-DE",
        "fr" => "fr-FR",
        "pt-BR" => "pt-BR",
        other => other,
    };

    let mut output = format!(
        "<speak version=\"1.1\" xmlns=\"http://www.w3.org/2001/10/synthesis\" xml:lang=\"{}\">\n",
        xml_escape(lang)
    );

    for segment in &script.segments {
        let rate = match segment.pace {
            Pace::Slow => "slow",
            Pace::Medium => "medium",
            Pace::Fast => "fast",
        };

        output.push_str(&format!("  <prosody rate=\"{rate}\">\n"));

        // Escape XML special chars first, then wrap emphasized words.
        // Emphasis uses word boundaries to avoid matching substrings inside other words.
        let escaped = xml_escape(&segment.text);
        let mut text = escaped.into_owned();
        for word in &segment.emphasis {
            if word.trim().is_empty() {
                continue;
            }
            let escaped_word = xml_escape(word);
            text = wrap_emphasis(&text, &escaped_word);
        }

        output.push_str(&format!("    {text}\n"));
        output.push_str("  </prosody>\n");

        if segment.pause_after_ms > 0 {
            output.push_str(&format!(
                "  <break time=\"{}ms\"/>\n",
                segment.pause_after_ms
            ));
        }
    }

    output.push_str("</speak>\n");
    output
}

/// XML-escape the five reserved characters. Returns a borrowed slice when no
/// escape is needed to avoid unnecessary allocations.
fn xml_escape(s: &str) -> std::borrow::Cow<'_, str> {
    if !s
        .chars()
        .any(|c| matches!(c, '<' | '>' | '&' | '"' | '\''))
    {
        return std::borrow::Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    std::borrow::Cow::Owned(out)
}

/// Wrap `word` in the emphasis tag, matching whole words only.
/// Avoids the substring-match pitfall (e.g. "the" inside "bathed").
fn wrap_emphasis(text: &str, word: &str) -> String {
    if word.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;
    let bytes = text.as_bytes();
    let word_bytes = word.as_bytes();
    let word_len = word_bytes.len();
    let mut i = 0;
    let is_word_char = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80;
    while i + word_len <= bytes.len() {
        if &bytes[i..i + word_len] == word_bytes {
            let prev_ok = i == 0 || !is_word_char(bytes[i - 1]);
            let next_ok = i + word_len == bytes.len() || !is_word_char(bytes[i + word_len]);
            if prev_ok && next_ok {
                out.push_str(&text[last..i]);
                out.push_str("<emphasis level=\"moderate\">");
                out.push_str(word);
                out.push_str("</emphasis>");
                i += word_len;
                last = i;
                continue;
            }
        }
        i += 1;
    }
    out.push_str(&text[last..]);
    out
}

// ── Time formatting helpers ──

fn format_srt_time(seconds: f64) -> String {
    let total_ms = (seconds * 1000.0) as u64;
    let h = total_ms / 3_600_000;
    let m = (total_ms % 3_600_000) / 60_000;
    let s = (total_ms % 60_000) / 1000;
    let ms = total_ms % 1000;
    format!("{h:02}:{m:02}:{s:02},{ms:03}")
}

fn format_vtt_time(seconds: f64) -> String {
    let total_ms = (seconds * 1000.0) as u64;
    let h = total_ms / 3_600_000;
    let m = (total_ms % 3_600_000) / 60_000;
    let s = (total_ms % 60_000) / 1000;
    let ms = total_ms % 1000;
    format!("{h:02}:{m:02}:{s:02}.{ms:03}")
}

fn format_human_time(seconds: f64) -> String {
    let m = (seconds / 60.0).floor() as u64;
    let s = seconds % 60.0;
    format!("{m}:{s:05.2}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;

    fn sample_script() -> NarrationScript {
        NarrationScript {
            title: "Test Video".to_string(),
            total_duration_seconds: 30.0,
            segments: vec![
                Segment {
                    index: 0,
                    start_seconds: 0.0,
                    end_seconds: 8.5,
                    text: "Welcome to the demo.".to_string(),
                    visual_description: "Title screen".to_string(),
                    emphasis: vec!["demo".to_string()],
                    pace: Pace::Slow,
                    pause_after_ms: 500,
                    frame_refs: vec![0],
                    voice_override: None,
                },
                Segment {
                    index: 1,
                    start_seconds: 8.5,
                    end_seconds: 20.0,
                    text: "Here we see the main interface.".to_string(),
                    visual_description: "Dashboard view".to_string(),
                    emphasis: vec![],
                    pace: Pace::Medium,
                    pause_after_ms: 300,
                    frame_refs: vec![1, 2],
                    voice_override: None,
                },
                Segment {
                    index: 2,
                    start_seconds: 20.0,
                    end_seconds: 30.0,
                    text: "Thank you for watching.".to_string(),
                    visual_description: "Closing screen".to_string(),
                    emphasis: vec![],
                    pace: Pace::Slow,
                    pause_after_ms: 0,
                    frame_refs: vec![3],
                    voice_override: None,
                },
            ],
            metadata: ScriptMetadata {
                style: "product_demo".to_string(),
                language: "en".to_string(),
                provider: "claude".to_string(),
                model: "claude-sonnet-4-20250514".to_string(),
                generated_at: "2026-04-03T14:00:00Z".to_string(),
            },
        }
    }

    #[test]
    fn test_format_srt_time() {
        assert_eq!(format_srt_time(0.0), "00:00:00,000");
        assert_eq!(format_srt_time(8.5), "00:00:08,500");
        assert_eq!(format_srt_time(65.123), "00:01:05,123");
        assert_eq!(format_srt_time(3661.5), "01:01:01,500");
    }

    #[test]
    fn test_format_vtt_time() {
        assert_eq!(format_vtt_time(0.0), "00:00:00.000");
        assert_eq!(format_vtt_time(8.5), "00:00:08.500");
    }

    #[test]
    fn test_format_human_time() {
        assert_eq!(format_human_time(0.0), "0:00.00");
        assert_eq!(format_human_time(65.0), "1:05.00");
        assert_eq!(format_human_time(125.5), "2:05.50");
    }

    #[test]
    fn test_export_srt() {
        let script = sample_script();
        let srt = export_srt(&script);
        assert!(srt.contains("1\n00:00:00,000 --> 00:00:08,500"));
        assert!(srt.contains("Welcome to the demo."));
        assert!(srt.contains("2\n00:00:08,500 --> 00:00:20,000"));
    }

    #[test]
    fn test_export_vtt() {
        let script = sample_script();
        let vtt = export_vtt(&script);
        assert!(vtt.starts_with("WEBVTT\n\n"));
        assert!(vtt.contains("00:00:00.000 --> 00:00:08.500"));
    }

    #[test]
    fn test_export_txt() {
        let script = sample_script();
        let txt = export_txt(&script);
        assert!(txt.contains("[0:00.00 - 0:08.50]"));
        assert!(txt.contains("Welcome to the demo."));
    }

    #[test]
    fn test_export_markdown() {
        let script = sample_script();
        let md = export_markdown(&script);
        assert!(md.contains("# Test Video"));
        assert!(md.contains("| # | Time | Text | Pace |"));
        assert!(md.contains("Welcome to the demo."));
    }

    #[test]
    fn test_export_ssml() {
        let script = sample_script();
        let ssml = export_ssml(&script);
        assert!(ssml.contains("<speak version=\"1.1\""));
        assert!(ssml.contains("xml:lang=\"en-US\""));
        assert!(ssml.contains("<prosody rate=\"slow\">"));
        assert!(ssml.contains("<emphasis level=\"moderate\">demo</emphasis>"));
        assert!(ssml.contains("<break time=\"500ms\"/>"));
    }

    #[test]
    fn test_export_json() {
        let script = sample_script();
        let json = export_json(&script);
        assert!(json.contains("\"title\": \"Test Video\""));
        assert!(json.contains("\"segments\""));
    }

    // ── Additional tests ──

    fn test_script() -> NarrationScript {
        NarrationScript {
            title: "Test".to_string(),
            total_duration_seconds: 30.0,
            segments: vec![
                Segment {
                    index: 0,
                    start_seconds: 0.0,
                    end_seconds: 10.0,
                    text: "First segment.".to_string(),
                    visual_description: "Opening".to_string(),
                    emphasis: vec![],
                    pace: Pace::Medium,
                    pause_after_ms: 500,
                    frame_refs: vec![0],
                    voice_override: None,
                },
                Segment {
                    index: 1,
                    start_seconds: 12.0,
                    end_seconds: 25.0,
                    text: "Second segment.".to_string(),
                    visual_description: "Main".to_string(),
                    emphasis: vec![],
                    pace: Pace::Fast,
                    pause_after_ms: 0,
                    frame_refs: vec![1],
                    voice_override: None,
                },
            ],
            metadata: ScriptMetadata {
                style: "technical".to_string(),
                language: "en".to_string(),
                provider: "claude".to_string(),
                model: "test".to_string(),
                generated_at: "2026-01-01T00:00:00Z".to_string(),
            },
        }
    }

    #[test]
    fn test_export_srt_with_test_script() {
        let script = test_script();
        let srt = export_srt(&script);
        // Verify SRT sequence numbers and timestamp format (comma separator)
        assert!(srt.contains("1\n00:00:00,000 --> 00:00:10,000"));
        assert!(srt.contains("2\n00:00:12,000 --> 00:00:25,000"));
        assert!(srt.contains("First segment."));
        assert!(srt.contains("Second segment."));
    }

    #[test]
    fn test_export_vtt_with_test_script() {
        let script = test_script();
        let vtt = export_vtt(&script);
        // VTT must start with WEBVTT header
        assert!(vtt.starts_with("WEBVTT\n\n"));
        // VTT uses dot separator for milliseconds
        assert!(vtt.contains("00:00:00.000 --> 00:00:10.000"));
        assert!(vtt.contains("00:00:12.000 --> 00:00:25.000"));
    }

    #[test]
    fn test_export_json_roundtrip() {
        let script = test_script();
        let json_str = export_json(&script);
        // Verify it's valid JSON by parsing it back
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["title"], "Test");
        assert_eq!(parsed["segments"].as_array().unwrap().len(), 2);
        assert_eq!(parsed["metadata"]["style"], "technical");
    }

    #[test]
    fn test_export_txt_with_test_script() {
        let script = test_script();
        let txt = export_txt(&script);
        // Verify human-readable time format
        assert!(txt.contains("[0:00.00 - 0:10.00]"));
        assert!(txt.contains("[0:12.00 - 0:25.00]"));
        assert!(txt.contains("First segment."));
        assert!(txt.contains("Second segment."));
    }

    #[test]
    fn test_export_markdown_with_test_script() {
        let script = test_script();
        let md = export_markdown(&script);
        // Title
        assert!(md.contains("# Test"));
        // Markdown table header
        assert!(md.contains("| # | Time | Text | Pace |"));
        assert!(md.contains("|---|------|------|------|"));
        // Metadata line
        assert!(md.contains("**Duration:** 30s"));
        assert!(md.contains("**Style:** technical"));
        // Segment data in table
        assert!(md.contains("First segment."));
        assert!(md.contains("medium"));
        assert!(md.contains("fast"));
    }

    #[test]
    fn test_export_ssml_with_test_script() {
        let script = test_script();
        let ssml = export_ssml(&script);
        // SSML structure
        assert!(ssml.starts_with("<speak version=\"1.1\""));
        assert!(ssml.contains("xml:lang=\"en-US\""));
        assert!(ssml.ends_with("</speak>\n"));
        // Prosody rates
        assert!(ssml.contains("<prosody rate=\"medium\">"));
        assert!(ssml.contains("<prosody rate=\"fast\">"));
        // Break after first segment (500ms pause)
        assert!(ssml.contains("<break time=\"500ms\"/>"));
        // No break after second segment (0ms pause)
        // Count occurrences of break — should be exactly 1
        let break_count = ssml.matches("<break time=").count();
        assert_eq!(break_count, 1);
    }

    #[test]
    fn test_export_empty_segments() {
        let script = NarrationScript {
            title: "Empty".to_string(),
            total_duration_seconds: 0.0,
            segments: vec![],
            metadata: ScriptMetadata {
                style: "technical".to_string(),
                language: "en".to_string(),
                provider: "claude".to_string(),
                model: "test".to_string(),
                generated_at: "2026-01-01T00:00:00Z".to_string(),
            },
        };

        // SRT: should be empty (no segments)
        let srt = export_srt(&script);
        assert!(srt.is_empty());

        // VTT: should only contain the header
        let vtt = export_vtt(&script);
        assert_eq!(vtt, "WEBVTT\n\n");

        // TXT: should be empty
        let txt = export_txt(&script);
        assert!(txt.is_empty());

        // JSON: should be valid and have empty segments array
        let json_str = export_json(&script);
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert!(parsed["segments"].as_array().unwrap().is_empty());

        // Markdown: should contain title and table header but no data rows
        let md = export_markdown(&script);
        assert!(md.contains("# Empty"));
        assert!(md.contains("| # | Time | Text | Pace |"));

        // SSML: should contain speak tags but no prosody
        let ssml = export_ssml(&script);
        assert!(ssml.contains("<speak"));
        assert!(ssml.contains("</speak>"));
        assert!(!ssml.contains("<prosody"));
    }

    // ── Unicode + special char edge cases ──────────────────────────────

    fn make_script(segments: Vec<(&str, Vec<&str>)>) -> NarrationScript {
        NarrationScript {
            title: "T".into(),
            total_duration_seconds: 30.0,
            segments: segments
                .into_iter()
                .enumerate()
                .map(|(i, (text, emph))| Segment {
                    index: i,
                    start_seconds: i as f64 * 3.0,
                    end_seconds: (i + 1) as f64 * 3.0,
                    text: text.into(),
                    visual_description: String::new(),
                    emphasis: emph.into_iter().map(String::from).collect(),
                    pace: Pace::Medium,
                    pause_after_ms: 0,
                    frame_refs: vec![],
                    voice_override: None,
                })
                .collect(),
            metadata: ScriptMetadata {
                style: "test".into(),
                language: "en".into(),
                provider: "mock".into(),
                model: "mock-v1".into(),
                generated_at: "2026-01-01T00:00:00Z".into(),
            },
        }
    }

    #[test]
    fn test_ssml_escapes_xml_special_chars() {
        let script = make_script(vec![("Save <file> & test", vec![])]);
        let ssml = export_ssml(&script);
        assert!(ssml.contains("Save &lt;file&gt; &amp; test"));
        // Must produce valid XML — no raw < inside prosody content
        assert!(!ssml.contains("Save <file>"));
    }

    #[test]
    fn test_ssml_emphasis_matches_whole_words_only() {
        // Emphasis "the" should NOT match inside "bathed" or "theory"
        let script = make_script(vec![("bathed theory of the system", vec!["the"])]);
        let ssml = export_ssml(&script);
        assert!(
            ssml.contains("of <emphasis level=\"moderate\">the</emphasis> system"),
            "standalone 'the' should be wrapped: {ssml}"
        );
        assert!(
            !ssml.contains("ba<emphasis"),
            "substring inside 'bathed' should NOT be wrapped: {ssml}"
        );
    }

    #[test]
    fn test_ssml_emphasis_empty_word_ignored() {
        let script = make_script(vec![("some text", vec!["", "   "])]);
        let ssml = export_ssml(&script);
        assert!(!ssml.contains("<emphasis"));
    }

    #[test]
    fn test_ssml_unicode_text_preserved() {
        let script = make_script(vec![("こんにちは 🎬 world", vec!["world"])]);
        let ssml = export_ssml(&script);
        assert!(ssml.contains("こんにちは"));
        assert!(ssml.contains("🎬"));
        assert!(ssml.contains("<emphasis level=\"moderate\">world</emphasis>"));
    }

    #[test]
    fn test_srt_format_handles_long_durations() {
        let t = format_srt_time(3661.500); // 1h 1m 1.5s
        assert_eq!(t, "01:01:01,500");
    }

    #[test]
    fn test_srt_format_zero_and_sub_second() {
        assert_eq!(format_srt_time(0.0), "00:00:00,000");
        assert_eq!(format_srt_time(0.5), "00:00:00,500");
    }

    #[test]
    fn test_vtt_format_uses_dot_separator() {
        assert_eq!(format_vtt_time(65.250), "00:01:05.250");
    }

    #[test]
    fn test_markdown_escapes_pipes() {
        let script = make_script(vec![("text with | pipe", vec![])]);
        let md = export_markdown(&script);
        assert!(md.contains("text with \\| pipe"));
    }

    #[test]
    fn test_srt_preserves_unicode_segment_text() {
        let script = make_script(vec![("日本語 セグメント", vec![])]);
        let srt = export_srt(&script);
        assert!(srt.contains("日本語 セグメント"));
    }

    #[test]
    fn test_xml_escape_borrowed_when_clean() {
        let input = "clean text";
        let output = xml_escape(input);
        // No escape needed → borrowed (no allocation)
        assert!(matches!(output, std::borrow::Cow::Borrowed(_)));
        assert_eq!(output, "clean text");
    }

    #[test]
    fn test_xml_escape_all_reserved() {
        let input = "<>&\"'";
        let output = xml_escape(input);
        assert_eq!(output, "&lt;&gt;&amp;&quot;&apos;");
    }

    #[test]
    fn test_wrap_emphasis_preserves_punctuation() {
        // Emphasis word next to punctuation should still match
        let out = wrap_emphasis("hello, world!", "world");
        assert_eq!(
            out,
            "hello, <emphasis level=\"moderate\">world</emphasis>!"
        );
    }

    #[test]
    fn test_wrap_emphasis_multiple_occurrences() {
        let out = wrap_emphasis("the cat and the dog", "the");
        assert_eq!(
            out,
            "<emphasis level=\"moderate\">the</emphasis> cat and <emphasis level=\"moderate\">the</emphasis> dog"
        );
    }
}
