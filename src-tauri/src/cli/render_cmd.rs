//! `narrator-cli render …` — invokes the render facade.

use clap::Subcommand;
use serde_json::{json, Value};

use crate::error::NarratorError;
use crate::render::{self, SubtitleStyle, VideoEditPlan};

use super::{build_reporter, read_json_arg, ProgressMode};

#[derive(Subcommand, Debug)]
pub enum RenderCmd {
    /// Apply an edit plan (clips + overlay effects) → single MP4.
    ApplyEdits {
        #[arg(long)]
        input: String,
        /// Path to a VideoEditPlan JSON file, or `-` for stdin.
        #[arg(long)]
        plan: String,
        #[arg(long)]
        output: String,
    },
    /// Burn an SRT into a video as hard-subtitles.
    BurnSubs {
        #[arg(long)]
        input: String,
        #[arg(long)]
        srt: String,
        #[arg(long)]
        output: String,
        /// Optional SubtitleStyle JSON file or `-` for stdin (defaults to standard style).
        #[arg(long)]
        style: Option<String>,
    },
    /// Mux audio onto a video. `--replace` swaps the audio track wholesale.
    MergeAudio {
        #[arg(long)]
        video: String,
        #[arg(long)]
        audio: String,
        #[arg(long)]
        output: String,
        #[arg(long, default_value_t = false)]
        replace: bool,
    },
    /// Extract a single frame at a given timestamp.
    ExtractFrame {
        #[arg(long)]
        input: String,
        #[arg(long)]
        at: f64,
        #[arg(long)]
        output: String,
    },
    /// Extract `count` evenly-spaced thumbnails into a directory.
    ExtractThumbnails {
        #[arg(long)]
        input: String,
        #[arg(long)]
        output_dir: String,
        #[arg(long, default_value_t = 12)]
        count: usize,
    },
}

pub async fn run(cmd: RenderCmd, progress: ProgressMode) -> Result<Value, NarratorError> {
    let reporter = build_reporter(progress);
    match cmd {
        RenderCmd::ApplyEdits {
            input,
            plan,
            output,
        } => {
            let plan: VideoEditPlan = read_json_arg(&plan)?;
            let path = render::apply_edits(&input, &output, &plan, reporter).await?;
            Ok(json!({ "output_path": path }))
        }
        RenderCmd::BurnSubs {
            input,
            srt,
            output,
            style,
        } => {
            let style: SubtitleStyle = match style.as_deref() {
                Some(p) => read_json_arg(p)?,
                None => SubtitleStyle::default(),
            };
            let path = render::burn_subtitles(&input, &srt, &output, &style, reporter).await?;
            Ok(json!({ "output_path": path }))
        }
        RenderCmd::MergeAudio {
            video,
            audio,
            output,
            replace,
        } => {
            let path =
                render::merge_audio_video(&video, &audio, &output, replace, reporter).await?;
            Ok(json!({ "output_path": path }))
        }
        RenderCmd::ExtractFrame { input, at, output } => {
            let path = render::extract_single_frame(&input, at, &output).await?;
            Ok(json!({ "output_path": path }))
        }
        RenderCmd::ExtractThumbnails {
            input,
            output_dir,
            count,
        } => {
            let paths = render::extract_edit_thumbnails(&input, &output_dir, count).await?;
            Ok(json!({ "paths": paths }))
        }
    }
}
