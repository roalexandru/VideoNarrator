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
        script.total_duration_seconds,
        script.metadata.style,
        script.metadata.language
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
        "<speak version=\"1.1\" xmlns=\"http://www.w3.org/2001/10/synthesis\" xml:lang=\"{lang}\">\n"
    );

    for segment in &script.segments {
        let rate = match segment.pace {
            Pace::Slow => "slow",
            Pace::Medium => "medium",
            Pace::Fast => "fast",
        };

        output.push_str(&format!("  <prosody rate=\"{rate}\">\n"));

        // Apply emphasis to marked words
        let mut text = segment.text.clone();
        for word in &segment.emphasis {
            text = text.replace(
                word,
                &format!("<emphasis level=\"moderate\">{word}</emphasis>"),
            );
        }

        output.push_str(&format!("    {text}\n"));
        output.push_str("  </prosody>\n");

        if segment.pause_after_ms > 0 {
            output.push_str(&format!("  <break time=\"{}ms\"/>\n", segment.pause_after_ms));
        }
    }

    output.push_str("</speak>\n");
    output
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
    let total_secs = seconds as u64;
    let m = total_secs / 60;
    let s = total_secs % 60;
    format!("{m}:{s:02}")
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
        assert_eq!(format_human_time(0.0), "0:00");
        assert_eq!(format_human_time(65.0), "1:05");
        assert_eq!(format_human_time(125.5), "2:05");
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
        assert!(txt.contains("[0:00 - 0:08]"));
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
}
