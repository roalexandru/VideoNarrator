//! `narrator-cli probe …` — read-only metadata queries.

use std::path::Path;

use clap::Subcommand;
use serde_json::{json, Value};

use crate::error::NarratorError;
use crate::render;

#[derive(Subcommand, Debug)]
pub enum ProbeCmd {
    /// Probe a video file for resolution / fps / duration / codec / size.
    Video {
        #[arg(long)]
        input: String,
    },
}

pub async fn run(cmd: ProbeCmd) -> Result<Value, NarratorError> {
    match cmd {
        ProbeCmd::Video { input } => {
            let meta = render::probe_video(Path::new(&input)).await?;
            Ok(json!(meta))
        }
    }
}
