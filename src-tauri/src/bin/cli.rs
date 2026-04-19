//! Entry point for the `narrator-cli` binary.
//!
//! Thin shell over `narrator_lib::cli::dispatch` so the actual command logic
//! lives in the library crate (and therefore stays testable as ordinary Rust
//! functions, not via spawning subprocesses).

use clap::Parser;
use narrator_lib::cli::{dispatch, Cli};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let code = dispatch(cli).await;
    std::process::exit(code);
}
