//! `narrator-cli` — headless surface for every backend capability.
//!
//! Subcommands mirror the IPC namespace (`render`, `probe`, …). Each command
//! takes JSON-in (`--input -` reads stdin) and prints a single JSON envelope
//! to stdout:
//!
//! ```json
//! {"ok": true,  "data": ...}
//! {"ok": false, "error": {"kind": "...", "message": "..."}}
//! ```
//!
//! With `--progress json`, intermediate progress events are written as one
//! NDJSON object per line to **stderr**, leaving stdout clean for the final
//! envelope so callers can pipe it through `jq` without filtering.
//!
//! The CLI links the same `narrator_lib` crate as the GUI; the renderer,
//! probes, and (in later phases) the AI / TTS clients are reused as-is.

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use serde::Serialize;
use serde_json::Value;

use crate::error::NarratorError;
use crate::models::ProgressEvent;
use crate::render::{self, ProgressReporter};

mod probe_cmd;
mod render_cmd;
mod tts_cmd;

#[derive(Parser, Debug)]
#[command(
    name = "narrator-cli",
    version,
    about = "Headless CLI for the Narrator video pipeline."
)]
pub struct Cli {
    /// How to emit progress events while a render is running.
    /// `none` suppresses; `json` writes one NDJSON event per line to stderr.
    #[arg(long, value_enum, default_value_t = ProgressMode::None, global = true)]
    pub progress: ProgressMode,

    #[command(subcommand)]
    pub command: TopCommand,
}

#[derive(Subcommand, Debug)]
pub enum TopCommand {
    /// Render operations: apply edits, burn subtitles, mux audio, extract frames.
    #[command(subcommand)]
    Render(render_cmd::RenderCmd),
    /// Probe a media file for metadata.
    #[command(subcommand)]
    Probe(probe_cmd::ProbeCmd),
    /// Text-to-speech operations using the builtin (free) TTS engine.
    #[command(subcommand)]
    Tts(tts_cmd::TtsCmd),
}

#[derive(Clone, Copy, Debug, Default, clap::ValueEnum)]
pub enum ProgressMode {
    #[default]
    None,
    Json,
}

/// Single-shape stdout envelope. Always one line of JSON.
#[derive(Serialize)]
#[serde(untagged)]
enum Envelope<T: Serialize> {
    Ok { ok: bool, data: T },
    Err { ok: bool, error: ErrorPayload },
}

#[derive(Serialize)]
struct ErrorPayload {
    kind: String,
    message: String,
}

impl<T: Serialize> Envelope<T> {
    fn ok(data: T) -> Self {
        Envelope::Ok { ok: true, data }
    }
}

fn err_envelope(e: &NarratorError) -> Envelope<Value> {
    Envelope::Err {
        ok: false,
        error: ErrorPayload {
            kind: format!("{:?}", e)
                .split('(')
                .next()
                .unwrap_or("Error")
                .to_string(),
            message: e.to_string(),
        },
    }
}

/// Print an envelope as exactly one line of JSON to stdout.
fn emit<T: Serialize>(env: Envelope<T>) {
    match serde_json::to_string(&env) {
        Ok(s) => println!("{s}"),
        Err(e) => {
            eprintln!("{{\"ok\":false,\"error\":{{\"kind\":\"Serde\",\"message\":\"{e}\"}}}}")
        }
    }
}

/// Build a progress reporter that honours `--progress`.
pub(crate) fn build_reporter(mode: ProgressMode) -> Arc<dyn ProgressReporter> {
    match mode {
        ProgressMode::None => Arc::new(render::NoopReporter),
        ProgressMode::Json => Arc::new(render::FnReporter(|event: ProgressEvent| {
            // One NDJSON object per line on stderr. Stdout stays reserved
            // for the final envelope so jq pipelines work cleanly.
            if let Ok(s) = serde_json::to_string(&event) {
                eprintln!("{s}");
            }
        })),
    }
}

/// Read a JSON value from a file path, or `-` for stdin.
pub(crate) fn read_json_arg<T: serde::de::DeserializeOwned>(
    path: &str,
) -> Result<T, NarratorError> {
    let raw = if path == "-" {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(NarratorError::IoError)?;
        buf
    } else {
        std::fs::read_to_string(PathBuf::from(path)).map_err(NarratorError::IoError)?
    };
    serde_json::from_str(&raw).map_err(NarratorError::from)
}

/// Top-level entry: parse args, dispatch, print one envelope, return exit code.
pub async fn dispatch(cli: Cli) -> i32 {
    let result = match cli.command {
        TopCommand::Render(cmd) => render_cmd::run(cmd, cli.progress).await,
        TopCommand::Probe(cmd) => probe_cmd::run(cmd).await,
        TopCommand::Tts(cmd) => tts_cmd::run(cmd).await,
    };
    match result {
        Ok(value) => {
            emit(Envelope::ok(value));
            0
        }
        Err(e) => {
            emit::<Value>(err_envelope(&e));
            1
        }
    }
}
