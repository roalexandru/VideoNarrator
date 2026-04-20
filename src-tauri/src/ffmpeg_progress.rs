//! Shared ffmpeg stderr progress parsing.
//!
//! ffmpeg can be driven to emit structured progress via `-progress pipe:2`
//! (lines like `out_time=HH:MM:SS.ms`). When that's not requested, legacy
//! `time=...` stats appear inside the normal stderr banner. We prefer the
//! former but fall back to the latter.
//!
//! Consumers: `video_edit::run_ffmpeg_with_progress` (invokes ffmpeg for the
//! single-clip fast paths and subtitle burn) and `video_engine::extract_frames`
//! (parses stderr while ffmpeg writes out sampled JPEGs). Keeping the parsers
//! here guarantees both behave identically.

/// Extract the time= value from an ffmpeg stderr line.
///
/// Handles both formats:
/// - Structured `-progress pipe:2` output: `out_time=00:00:01.666000\n`
///   (and `out_time_us=...`, `out_time_ms=...` — we prefer `out_time=`).
/// - Legacy `-stats` output: `frame=100 ... time=00:00:01.66 bitrate=...\r`.
pub fn extract_time_from_ffmpeg_line(line: &str) -> Option<String> {
    // Prefer the structured `-progress` format; it's \n-terminated so lines()
    // can actually see it.
    if let Some(i) = line.find("out_time=") {
        let rest = &line[i + 9..];
        let end = rest.find([' ', '\n', '\r']).unwrap_or(rest.len());
        let time_str = rest[..end].trim();
        if time_str.is_empty() || time_str == "N/A" {
            return None;
        }
        return Some(time_str.to_string());
    }
    // Fallback: legacy stats line. This only fires in contexts where we didn't
    // pass -nostats.
    let time_idx = line.find("time=")?;
    let rest = &line[time_idx + 5..];
    let end = rest.find(' ').unwrap_or(rest.len());
    let time_str = &rest[..end];
    if time_str == "N/A" {
        return None;
    }
    Some(time_str.to_string())
}

/// Parse ffmpeg time format "HH:MM:SS.ms" (or "MM:SS.ms", or plain seconds) to
/// seconds as f64. Returns 0.0 for malformed input rather than panicking so a
/// single bad line can't knock out progress reporting.
pub fn parse_ffmpeg_time(time_str: &str) -> f64 {
    let parts: Vec<&str> = time_str.split(':').collect();
    match parts.len() {
        3 => {
            let hours: f64 = parts[0].parse().unwrap_or(0.0);
            let minutes: f64 = parts[1].parse().unwrap_or(0.0);
            let seconds: f64 = parts[2].parse().unwrap_or(0.0);
            hours * 3600.0 + minutes * 60.0 + seconds
        }
        2 => {
            let minutes: f64 = parts[0].parse().unwrap_or(0.0);
            let seconds: f64 = parts[1].parse().unwrap_or(0.0);
            minutes * 60.0 + seconds
        }
        1 => parts[0].parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_time_from_ffmpeg_line() {
        assert_eq!(
            extract_time_from_ffmpeg_line("frame=100 time=00:00:10.50 size=1024kB"),
            Some("00:00:10.50".to_string())
        );
        assert_eq!(extract_time_from_ffmpeg_line("time=N/A"), None);
        assert_eq!(extract_time_from_ffmpeg_line("no time here"), None);
    }

    #[test]
    fn test_extract_time_from_progress_pipe_format() {
        assert_eq!(
            extract_time_from_ffmpeg_line("out_time=00:00:01.666000"),
            Some("00:00:01.666000".to_string())
        );
        // `out_time=N/A` happens during the initial frames and must be dropped.
        assert_eq!(extract_time_from_ffmpeg_line("out_time=N/A"), None);
    }

    #[test]
    fn test_parse_ffmpeg_time() {
        assert!((parse_ffmpeg_time("00:01:30.50") - 90.5).abs() < 0.01);
        assert!((parse_ffmpeg_time("01:00:00.00") - 3600.0).abs() < 0.01);
        assert!((parse_ffmpeg_time("00:00:05.25") - 5.25).abs() < 0.01);
        assert!((parse_ffmpeg_time("") - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_parse_ffmpeg_time_two_parts() {
        assert!((parse_ffmpeg_time("01:30.00") - 90.0).abs() < 0.01);
    }

    #[test]
    fn test_extract_time_with_size_prefix() {
        assert_eq!(
            extract_time_from_ffmpeg_line("size=1024kB time=00:00:10.00"),
            Some("00:00:10.00".to_string())
        );
    }
}
